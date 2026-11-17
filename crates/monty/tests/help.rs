//! Plain-text help intentionally uses a smaller layout than CPython's pydoc.

use std::borrow::Cow;

use insta::assert_snapshot;
use monty::{Dump, MontyRepl, MontyRun, Session, SessionRef, dump};
use monty_types::{
    CollectedStreams, CompileOptions, ExcType, ExtFunctionResult, ModuleFile, MontyException, MontyObject, MontyUuid,
    PrintStream, PrintWriter, PrintWriterCallback, ResourceTracker,
};

fn capture(code: &str) -> String {
    capture_inputs(code, vec![], vec![])
}

fn capture_inputs(code: &str, names: Vec<String>, inputs: Vec<MontyObject>) -> String {
    let mut runner = MontyRun::new(code.to_owned(), "help.py", names, CompileOptions::default()).unwrap();
    let mut streams = CollectedStreams::default();
    let result = runner
        .run(
            inputs,
            ResourceTracker::default(),
            PrintWriter::collect_streams(&mut streams),
        )
        .unwrap();
    assert_eq!(result, MontyObject::none());
    let streams = streams.into_entries();
    assert_eq!(streams.len(), 1);
    assert_eq!(streams[0].0, PrintStream::Stdout);
    streams.into_iter().next().unwrap().1
}

#[test]
fn parameter_kinds_defaults_and_async() {
    assert_snapshot!(capture(
        r#"
default = [1]
def all_kinds(a, b=2, /, c=3, *items, required, optional='value', another, **options):
    """Call the tool."""
def defaults(a=default, /, b=None, *, flag=True):
    pass
default.append(2)
default = 'rebound'
async def async_tool(query: MissingType, *, limit=10) -> AnotherMissingType:
    """Fetch results."""
def keyword_only(*, required, optional=4, last):
    pass
assert help(all_kinds) is None
help(defaults)
help(async_tool)
help(keyword_only)
help(lambda x=5, **kwargs: x)
"#
    ));
}

#[test]
fn closures_keep_their_own_defaults_and_documentation() {
    assert_snapshot!(capture(
        r#"
def factory(value):
    def tool(a=value, *, b=value + 1):
        """Original."""
        return value
    return tool
first = factory(2)
second = factory(20)
alias = first
first.__doc__ = 'Updated through an alias.'
help(alias)
help(second)
"#
    ));
}

#[test]
fn bound_methods_omit_only_the_receiver() {
    assert_snapshot!(capture(
        r#"
class Tool:
    def positional(self, value=1, /, *, mode='read'):
        """Original."""
    def sole(self, /, value=2):
        pass
    def ordinary(self, value=3):
        pass
    def variadic(*args, **kwargs):
        pass
    def empty():
        pass
    async def fetch(self, *, limit=4):
        pass
tool = Tool()
method = tool.positional
Tool.positional.__doc__ = 'Updated bound documentation.'
help(method)
help(Tool.positional)
help(tool.sole)
help(tool.ordinary)
help(tool.variadic)
help(tool.empty)
help(tool.fetch)
"#
    ));
}

#[test]
fn class_and_instance_help_list_public_members_without_recursing() {
    assert_snapshot!(capture(
        r#"
class Tool:
    """A documented tool."""
    version = 2
    _private = 3
    annotation: MissingType
    def __init__(self, name, *, retries=2):
        """Configure the tool."""
    def __call__(self, query):
        """Run a query."""
    async def fetch(self, query, *, limit=5):
        """Fetch results."""
    def _hidden(self):
        pass
Tool.again = Tool
help(Tool)
help(Tool('search'))
class Empty:
    pass
help(Empty)
"#
    ));
}

#[test]
fn class_constructors_preserve_bound_initializer_signatures() {
    let output = capture(
        r#"
class Initializer:
    def setup(self, value=1, /, *, flag=True):
        """Configure the owner."""
        self.value = value
        self.flag = flag
    def keyword_only(self, *, limit=2):
        self.limit = limit

owner = Initializer()
class Tool:
    pass
Tool.__init__ = owner.setup
Tool(3, flag=False)
assert owner.value == 3
assert owner.flag is False
help(Tool.__init__)
help(Tool)

class KeywordTool:
    __init__ = owner.keyword_only
KeywordTool(limit=4)
assert owner.limit == 4
help(KeywordTool.__init__)
help(KeywordTool)
"#,
    );
    assert_snapshot!(output);
}

#[test]
fn docstring_indentation_and_non_string_docs() {
    assert_snapshot!(capture(
        r"
def tool():
    pass
tool.__doc__ = '\n\tSummary.\n\n\tExample:\n\t    tool()\n\t\n'
help(tool)
tool.__doc__ = 'Résumé.\n\u2003\u2003Indented.\n\u2003\u2003  More.'
help(tool)
tool.__doc__ = '    First line.\n        Second line.\n            Nested.'
help(tool)
tool.__doc__ = None
help(tool)
tool.__doc__ = ['not a docstring']
help(tool)
tool.__doc__ = tool
help(tool)
"
    ));
}

#[test]
fn member_aliases_and_mutation_during_formatting() {
    assert_snapshot!(capture(
        r#"
class Tools:
    """Tools available to this session."""
tools = Tools
class Default:
    def __repr__(self):
        tools.changed = 1
        tools.z_tool = None
        return '<default>'
def tool(value=Default(), *, mode='fast'):
    """Run this tool."""
tools.a_tool = tool
tools.z_tool = tool
tools._private = tool
tools.again = tools
help(tools)
"#
    ));
}

#[test]
fn defaults_can_change_docs_or_raise_without_leaking_references() {
    assert_snapshot!(capture(
        r#"
class Default:
    def __repr__(self):
        tool.__doc__ = 'Changed while formatting.'
        return '<default>'
def tool(value=Default()):
    """Before formatting."""
help(tool)
help(tool)
class Broken:
    def __repr__(self):
        raise ValueError('cannot represent default')
def broken(value=Broken()):
    pass
try:
    help(broken)
except ValueError as exc:
    assert str(exc) == 'cannot represent default'
else:
    assert False, 'expected a repr error'
help(tool)
"#
    ));
}

#[test]
fn file_module_docs_and_functions() {
    let code = "import tools\nhelp(tools)\ntools.__doc__ = 'Updated module.'\nhelp(tools)";
    let runner = MontyRun::new(code.to_owned(), "help.py", vec![], CompileOptions::default()).unwrap();
    let mut output = String::new();
    let call = runner
        .start(
            vec![],
            ResourceTracker::default(),
            PrintWriter::collect_string(&mut output),
        )
        .unwrap()
        .into_os_call()
        .unwrap();
    let source = r#"
"""Tools from a Python file."""
def run(query, /, *, limit=2):
    """Search for a query."""
_hidden = 1
alias = run
"#;
    let result = call
        .resume(
            ExtFunctionResult::ModuleFile(ModuleFile {
                path: "/tools.py".to_owned(),
                source: source.to_owned(),
            }),
            PrintWriter::collect_string(&mut output),
        )
        .unwrap();
    assert_eq!(result.into_complete(), Some(MontyObject::none()));
    assert_snapshot!(output);
}

#[test]
fn unknown_signatures_and_eager_host_metadata() {
    let class = MontyObject::class_type(
        "Tools",
        MontyUuid::from_u128(1),
        true,
        false,
        [
            (
                MontyObject::string("__doc__"),
                MontyObject::string("Host tool collection."),
            ),
            (
                MontyObject::string("run"),
                MontyObject::function("tools.run", Some("Run the host tool.".to_owned())),
            ),
        ],
    );
    let module = MontyObject::module(
        "tools",
        MontyUuid::from_u128(2),
        [
            (
                MontyObject::string("__doc__"),
                MontyObject::string("Host module documentation."),
            ),
            (
                MontyObject::string("fetch"),
                MontyObject::async_function("tools.fetch", Some("Fetch results from the host.".to_owned())),
            ),
        ],
    );
    assert_snapshot!(capture_inputs(
        "help(Tools)\nhelp(module)\nhelp(module.fetch)\nhelp(len)\nhelp(int)\nhelp(None)",
        vec!["Tools".to_owned(), "module".to_owned()],
        vec![class, module],
    ));
}

#[test]
fn builtin_module_help_does_not_resolve_host_properties() {
    let output = capture("import os\nhelp(os)");
    assert_snapshot!(output);
}

#[test]
fn help_and_metadata_survive_repl_dumps() {
    let mut repl = MontyRepl::new("help.py", ResourceTracker::default(), CompileOptions::default());
    repl.feed_run(
        r#"
defaults = [1]
class Tool:
    async def run(self, value=defaults, *, mode='fast'):
        """Original documentation."""
method = Tool().run
show = help
"#,
        vec![],
        PrintWriter::Disabled,
    )
    .unwrap();
    let bytes = dump("help.py", None, &[], SessionRef::Idle(&repl)).unwrap();
    let Session::Idle(mut restored) = Dump::load(&bytes).unwrap().state else {
        panic!("expected idle session");
    };
    let mut output = String::new();
    restored
        .feed_run(
            "defaults.append(2)\nTool.run.__doc__ = 'Updated after restore.'\nshow(method)",
            vec![],
            PrintWriter::collect_string(&mut output),
        )
        .unwrap();
    assert_snapshot!(output);
}

#[test]
fn unsupported_calls_fail_without_interaction() {
    let mut errors = Vec::new();
    for code in ["help()", "help(len, int)", "help(object=len)", "help('topics')"] {
        let mut runner = MontyRun::new(code.to_owned(), "help.py", vec![], CompileOptions::default()).unwrap();
        let err = runner
            .run(vec![], ResourceTracker::default(), PrintWriter::Disabled)
            .unwrap_err();
        errors.push(err.to_string());
    }
    assert_snapshot!(errors.join("\n"));
}

#[test]
fn writer_errors_propagate() {
    struct FailingWriter;
    impl PrintWriterCallback for FailingWriter {
        fn stdout_write(&mut self, _: Cow<'_, str>) -> Result<(), MontyException> {
            Err(MontyException::new(
                ExcType::ValueError,
                Some("writer failed".to_owned()),
            ))
        }

        fn stdout_push(&mut self, _: char) -> Result<(), MontyException> {
            unreachable!("help writes a complete string");
        }
    }
    let mut runner = MontyRun::new("help(len)".to_owned(), "help.py", vec![], CompileOptions::default()).unwrap();
    let err = runner
        .run(
            vec![],
            ResourceTracker::default(),
            PrintWriter::Callback(&mut FailingWriter),
        )
        .unwrap_err();
    assert_snapshot!(err.to_string());
}
