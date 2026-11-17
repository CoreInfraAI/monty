//! Identity, equality, and export semantics for external function inputs (#347, #345).
//!
//! External functions are heap values cached by lookup name. Host object identity is
//! unavailable across the subprocess boundary, so live functions with the same name
//! share sandbox identity regardless of which conversion path produced them.

use monty::{Dump, MontyRepl, MontyRun, RunProgress, Session, SessionRef, dump};
use monty_types::{CompileOptions, MontyObject, NameLookupResult, PrintWriter, ResourceTracker};

/// Metadata is discoverable and survives export, including empty and missing docstrings.
#[test]
fn host_function_metadata_round_trips() {
    for doc in [
        None,
        Some(""),
        Some("Search for café locations.\n\nReturns matching places."),
    ] {
        for is_async in [false, true] {
            let input = if is_async {
                MontyObject::async_function("tools.search", doc.map(str::to_owned))
            } else {
                MontyObject::function("tools.search", doc.map(str::to_owned))
            };
            let mut runner = MontyRun::new(
                r"
assert f.__name__ == 'tools.search'
assert getattr(f, '__doc__') == expected_doc
assert hasattr(f, '__doc__')
assert dir(f) == ['__doc__', '__name__']
alias = f
assert alias.__doc__ == expected_doc
f
"
                .to_owned(),
                "test.py",
                vec!["f".to_owned(), "expected_doc".to_owned()],
                CompileOptions::default(),
            )
            .unwrap();
            let expected_doc = doc.map_or_else(MontyObject::none, MontyObject::string);
            assert_eq!(runner.run_no_limits(vec![input.clone(), expected_doc]).unwrap(), input);
        }
    }
}

/// Metadata belongs to the cached function, while sync and async proxies remain separate.
#[test]
fn host_doc_updates_reach_aliases_and_survive_dumps() {
    let mut repl = MontyRepl::new("test.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        "alias = f\nkey = {f: 'retained'}",
        vec![(
            "f".to_owned(),
            MontyObject::function("search", Some("Original.".to_owned())),
        )],
        PrintWriter::Disabled,
    )
    .unwrap();
    let bytes = dump("test.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut repl) = Dump::load(&bytes).unwrap().state else {
        panic!("expected idle session");
    };
    let result = repl
        .feed_run(
            r"
assert f.__doc__ == 'Original.'
assert f.__name__ == 'search'
assert f is alias
f
",
            vec![],
            PrintWriter::Disabled,
        )
        .unwrap();
    assert_eq!(result, MontyObject::function("search", Some("Original.".to_owned())));
    for doc in [Some("Updated."), Some(""), None] {
        let result = repl
            .feed_run(
                r"
assert f is alias is incoming
assert key[incoming] == 'retained'
assert async_f is not f
assert async_f.__doc__ == 'Async documentation.'
assert alias.__doc__ == incoming.__doc__
f
",
                vec![
                    (
                        "incoming".to_owned(),
                        MontyObject::function("search", doc.map(str::to_owned)),
                    ),
                    (
                        "async_f".to_owned(),
                        MontyObject::async_function("search", Some("Async documentation.".to_owned())),
                    ),
                ],
                PrintWriter::Disabled,
            )
            .unwrap();
        assert_eq!(result, MontyObject::function("search", doc.map(str::to_owned)));
    }
}

/// Lookup replies and callback results use the same metadata conversion as inputs.
#[test]
fn resolved_and_returned_host_functions_keep_docs() {
    let runner = MontyRun::new(
        "f = lookup\nassert f.__doc__ == 'From lookup.'\ng = f()\nassert g.__doc__ == 'From result.'\ng".to_owned(),
        "test.py",
        vec![],
        CompileOptions::default(),
    )
    .unwrap();
    let lookup = runner
        .start(vec![], ResourceTracker::default(), PrintWriter::Disabled)
        .unwrap()
        .into_name_lookup()
        .unwrap();
    let call = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("fetch", Some("From lookup.".to_owned()))),
            PrintWriter::Disabled,
        )
        .unwrap()
        .into_function_call()
        .unwrap();
    assert_eq!(call.function_name, "fetch");
    let result = MontyObject::function("result", Some("From result.".to_owned()));
    assert_eq!(
        call.resume(result.clone(), PrintWriter::Disabled)
            .unwrap()
            .into_complete(),
        Some(result)
    );
}

/// Sandbox writes are rejected; reading metadata does not make it callable.
#[test]
fn host_function_metadata_is_read_only() {
    let mut runner = MontyRun::new(
        r#"
for name in ['__doc__', '__name__']:
    try:
        setattr(f, name, ['replacement'])
    except AttributeError:
        pass
    else:
        assert False, 'expected read-only metadata'
assert f.__name__ == 'search'
assert f.__doc__ == 'Original.'
for call in [lambda: f.__doc__([1]), lambda: f.__name__(value=[2])]:
    try:
        call()
    except TypeError as exc:
        assert str(exc) == "'str' object is not callable"
    else:
        assert False, 'expected non-callable metadata'
try:
    empty.__doc__([3])
except TypeError as exc:
    assert str(exc) == "'NoneType' object is not callable"
else:
    assert False, 'expected non-callable metadata'
try:
    f.missing([4])
except AttributeError:
    pass
else:
    assert False, 'expected missing attribute'
assert getattr(f, 'missing', None) is None
f
"#
        .to_owned(),
        "test.py",
        vec!["f".to_owned(), "empty".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let input = MontyObject::function("search", Some("Original.".to_owned()));
    assert_eq!(
        runner
            .run_no_limits(vec![input.clone(), MontyObject::function("empty", None)])
            .unwrap(),
        input
    );
}

/// Builds two `MontyObject::function` inputs with the same `__name__` ("foo")
/// and runs `code` against them as inputs `a` and `b`.
fn run_with_same_named_callable_inputs(code: &str) -> MontyObject {
    let mut runner = MontyRun::new(
        code.to_owned(),
        "test.py",
        vec!["a".to_owned(), "b".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    runner
        .run_no_limits(vec![
            MontyObject::function("foo".to_owned(), None),
            MontyObject::function("foo".to_owned(), None),
        ])
        .unwrap()
}

/// Separate conversions of a live lookup name reuse its function object.
#[test]
fn same_named_callables_share_identity() {
    let result = run_with_same_named_callable_inputs("(a is b, a == b, id(a) == id(b))");
    assert_eq!(
        result,
        MontyObject::tuple([
            MontyObject::bool(true),
            MontyObject::bool(true),
            MontyObject::bool(true),
        ]),
    );
}

/// Mentioning the host function's name in source does not change its identity.
#[test]
fn same_named_callables_share_identity_when_name_is_interned() {
    let result = run_with_same_named_callable_inputs("foo = None\n(a is b, a == b)");
    assert_eq!(
        result,
        MontyObject::tuple([MontyObject::bool(true), MontyObject::bool(true)]),
    );
}

/// The same live external function occupies one dictionary key.
#[test]
fn same_named_callables_share_dict_key() {
    let result = run_with_same_named_callable_inputs("d = {a: 42, b: 43}\n(len(d), d[a], d[b])");
    assert_eq!(
        result,
        MontyObject::tuple([MontyObject::int(1), MontyObject::int(43), MontyObject::int(43)]),
    );
}

/// Functions with different names are also distinct objects.
#[test]
fn different_named_callables_remain_distinct() {
    let mut runner = MontyRun::new(
        "(a is b, a == b, id(a) == id(b))".to_owned(),
        "test.py",
        vec!["a".to_owned(), "b".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let result = runner
        .run_no_limits(vec![
            MontyObject::function("foo".to_owned(), None),
            MontyObject::function("bar".to_owned(), None),
        ])
        .unwrap();
    assert_eq!(
        result,
        MontyObject::tuple([
            MontyObject::bool(false),
            MontyObject::bool(false),
            MontyObject::bool(false),
        ]),
    );
}

/// An external function exports as a `MontyNode::Function` even when its name
/// also appears in source.
#[test]
fn callable_exports_as_function_object() {
    let mut runner = MontyRun::new(
        "foo = None\nx".to_owned(),
        "test.py",
        vec!["x".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let result = runner
        .run_no_limits(vec![MontyObject::function("foo".to_owned(), None)])
        .unwrap();
    assert_eq!(result, MontyObject::function("foo".to_owned(), None),);
}

/// Same callable, same export, regardless of source mention.
#[test]
fn callable_export_stable_across_source_mention() {
    let func_input = || vec![MontyObject::function("foo".to_owned(), None)];
    let r1 = MontyRun::new(
        "x".to_owned(),
        "test.py",
        vec!["x".to_owned()],
        CompileOptions::default(),
    )
    .unwrap()
    .run_no_limits(func_input())
    .unwrap();
    let r2 = MontyRun::new(
        "foo = None\nx".to_owned(),
        "test.py",
        vec!["x".to_owned()],
        CompileOptions::default(),
    )
    .unwrap()
    .run_no_limits(func_input())
    .unwrap();
    assert_eq!(r1, r2);
}

/// A live external function retains object identity across REPL feeds.
#[test]
fn repl_extfunction_identity_across_feeds() {
    let repl = MontyRepl::new("session.py", ResourceTracker::default(), CompileOptions::default());

    // Feed 1 resolves "foobar" to a host function named "ext_fn".
    let progress = repl.feed_start("x = foobar", vec![], PrintWriter::Stdout).unwrap();
    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'foobar'");
    assert_eq!(lookup.name, "foobar");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (repl, _) = progress.into_complete().expect("feed 1 should complete");

    // Feed 2 resolves another name to a function with the same lookup name.
    let progress = repl
        .feed_start(
            "ext_fn = 1\ny = barbaz\n(x is y, x == y, id(x) == id(y), hash(x) == hash(y))",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'barbaz'");
    assert_eq!(lookup.name, "barbaz");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (_repl, result) = progress.into_complete().expect("feed 2 should complete");

    // The second conversion reuses the live function object cached by name.
    assert_eq!(
        result,
        MontyObject::tuple([
            MontyObject::bool(true),
            MontyObject::bool(true),
            MontyObject::bool(true),
            MontyObject::bool(true),
        ]),
    );
}

/// Snapshot loading rebuilds the weak cache from live external functions.
#[test]
fn extfunction_cache_is_rebuilt_after_snapshot_load() {
    let runner = MontyRun::new(
        "gate()\ny = missing\nx is y".to_owned(),
        "test.py",
        vec!["x".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let progress = runner
        .start(
            vec![MontyObject::function("ext_fn".to_owned(), None)],
            ResourceTracker::default(),
            PrintWriter::Stdout,
        )
        .unwrap();
    let bytes = dump("test.py", None, &[], SessionRef::Running(&progress)).unwrap();
    assert_eq!(resume_snapshot_identity_test(progress), MontyObject::bool(true));

    let Session::Running(progress) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped a running session")
    };
    let progress = *progress;
    assert_eq!(resume_snapshot_identity_test(progress), MontyObject::bool(true));
}

/// Completes the snapshot cache test while preserving refcount cleanup.
fn resume_snapshot_identity_test(progress: RunProgress) -> MontyObject {
    let call = progress.into_function_call().expect("expected call to 'gate'");
    assert_eq!(call.function_name, "gate");

    let progress = call.resume(MontyObject::none(), PrintWriter::Stdout).unwrap();
    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'missing'");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    progress.into_complete().unwrap()
}

/// Dropping the last reference removes the weak-cache entry before slot reuse.
#[test]
fn repl_extfunction_cache_does_not_retain_freed_id() {
    let repl = MontyRepl::new("session.py", ResourceTracker::default(), CompileOptions::default());

    let progress = repl.feed_start("x = foobar", vec![], PrintWriter::Stdout).unwrap();
    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'foobar'");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (repl, _) = progress.into_complete().expect("feed 1 should complete");

    let progress = repl
        .feed_start("x = None\noccupied = []", vec![], PrintWriter::Stdout)
        .unwrap();
    let (repl, _) = progress.into_complete().expect("feed 2 should complete");

    let progress = repl
        .feed_start(
            "y = barbaz\n(y is occupied, y == occupied)",
            vec![],
            PrintWriter::Stdout,
        )
        .unwrap();
    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'barbaz'");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    let (_repl, result) = progress.into_complete().expect("feed 3 should complete");

    assert_eq!(
        result,
        MontyObject::tuple([MontyObject::bool(false), MontyObject::bool(false)]),
    );
}

/// Cycle collection removes weak entries for external-function children.
#[cfg(feature = "test-hooks")]
#[test]
fn gc_freed_ext_function_does_not_retain_freed_id() {
    let code = "\
import gc
d = {}
d['f'] = a
d['self'] = d
d = None
a = None
gc.collect()
occupied = [1, 2, 3]
y = missing
(y is occupied, repr(y))
";
    let runner = MontyRun::new(
        code.to_owned(),
        "test.py",
        vec!["a".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let progress = runner
        .start(
            vec![MontyObject::function("ext_fn".to_owned(), None)],
            ResourceTracker::default(),
            PrintWriter::Stdout,
        )
        .unwrap();

    let lookup = progress.into_name_lookup().expect("expected NameLookup for 'missing'");
    assert_eq!(lookup.name, "missing");
    let progress = lookup
        .resume(
            NameLookupResult::Value(MontyObject::function("ext_fn".to_owned(), None)),
            PrintWriter::Stdout,
        )
        .unwrap();
    let result = progress.into_complete().expect("run should complete");

    assert_eq!(
        result,
        MontyObject::tuple([
            MontyObject::bool(false),
            MontyObject::string("<function 'ext_fn' external>".to_owned()),
        ]),
    );
}

/// A sync and an async host function with one name stay distinct objects, and
/// each keeps its kind through a dump: the async flag is stored with the name.
#[test]
fn async_flag_survives_dump_and_splits_identity() {
    let runner = MontyRun::new(
        "gate()\n(a is b, a, b)".to_owned(),
        "test.py",
        vec!["a".to_owned(), "b".to_owned()],
        CompileOptions::default(),
    )
    .unwrap();
    let progress = runner
        .start(
            vec![
                MontyObject::function("foo".to_owned(), None),
                MontyObject::async_function("foo".to_owned(), None),
            ],
            ResourceTracker::default(),
            PrintWriter::Stdout,
        )
        .unwrap();
    let bytes = dump("test.py", None, &[], SessionRef::Running(&progress)).unwrap();
    let expected = MontyObject::tuple([
        MontyObject::bool(false),
        MontyObject::function("foo".to_owned(), None),
        MontyObject::async_function("foo".to_owned(), None),
    ]);
    assert_eq!(resume_gate(progress), expected);

    let Session::Running(progress) = Dump::load(&bytes).unwrap().state else {
        panic!("dumped a running session")
    };
    assert_eq!(resume_gate(*progress), expected);
}

/// Answers the `gate()` call and returns the run's result.
fn resume_gate(progress: RunProgress) -> MontyObject {
    let call = progress.into_function_call().expect("expected call to 'gate'");
    let progress = call.resume(MontyObject::none(), PrintWriter::Stdout).unwrap();
    progress.into_complete().unwrap()
}
