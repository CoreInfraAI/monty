//! Non-interactive help from stored documentation and callable metadata.

use std::{iter, mem};

use monty_types::PrintStream;

use super::Builtins;
use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, HeapData},
    string_builder::StringBuilder,
    types::Dict,
    value::{VALUE_SIZE, Value},
};

struct Description {
    kind: &'static str,
    name: String,
    signature: Option<String>,
    doc: Option<String>,
    callable: bool,
    is_async: bool,
}

/// Writes to the configured stdout stream and returns `None`; never starts a pager or prompt.
pub fn builtin_help(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let object = args.reject_kwargs("help", vm.heap)?.get_one_arg("help", vm.heap)?;
    defer_drop!(object, vm);
    if object.is_str(vm.heap) {
        return Err(ExcType::not_implemented("help() string topics are not supported").into());
    }
    // As in Python, help on an instance describes its class.
    let class_id = match object {
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Instance(instance) => Some(instance.class()),
            HeapData::HostClass(instance) => Some(instance.class_id()),
            _ => None,
        },
        _ => None,
    };
    let target = class_id.map_or_else(
        || object.clone_with_heap(vm.heap),
        |id| {
            vm.heap.inc_ref(id);
            Value::Ref(id)
        },
    );
    defer_drop!(target, vm);
    let description = describe(target, false, vm)?;
    let mut builder = StringBuilder::new(&vm.heap.tracker);
    builder.push_str("Help on ")?;
    builder.push_str(description.kind)?;
    builder.push(' ')?;
    builder.push_str(&description.name)?;
    builder.push_str(":\n\n")?;
    let mut output = builder.finish_raw()?;
    append_description(&mut output, &description.name, &description, false, vm)?;

    let members = members(target, vm)?;
    defer_drop!(members, vm);
    if !members.is_empty() {
        let mut builder = StringBuilder::from_existing(output, &vm.heap.tracker);
        builder.push_str("\nMembers:\n")?;
        output = builder.finish_raw()?;
    }
    for (index, (name, value)) in members.iter().enumerate() {
        vm.heap.tracker.check_time_every(index)?;
        let name = name.as_str(vm).expect("member names are strings").to_owned();
        let description = describe(value, false, vm)?;
        append_description(&mut output, &name, &description, true, vm)?;
    }
    vm.print_writer.write(PrintStream::Stdout, output.into())?;
    Ok(Value::None)
}

fn describe(object: &Value, bound: bool, vm: &mut VM<'_>) -> RunResult<Description> {
    if let Some(description) = function_description(object, bound, vm)? {
        return Ok(description);
    }
    let mut description = Description {
        kind: "object",
        name: object.py_type_name(vm).into_owned(),
        signature: None,
        doc: None,
        callable: object.is_callable(vm),
        is_async: false,
    };
    match object {
        Value::Builtin(builtin) => {
            let (kind, name) = match builtin {
                Builtins::Function(function) => ("built-in function", function.to_string()),
                Builtins::Type(ty) => ("built-in class", ty.to_string()),
                Builtins::ExcType(ty) => ("exception class", ty.to_string()),
            };
            description.kind = kind;
            description.name = name;
        }
        Value::ModuleFunction(function) => {
            description.kind = "built-in function";
            description.name = function.to_string();
        }
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::BoundMethod(method) => {
                let function = method.func.clone_with_heap(vm.heap);
                defer_drop!(function, vm);
                let mut description = describe(function, true, vm)?;
                description.kind = if description.is_async { "async method" } else { "method" };
                return Ok(description);
            }
            HeapData::ExtFunction(function) => {
                description.kind = "external function";
                function.as_str().clone_into(&mut description.name);
                description.is_async = function.is_async();
            }
            HeapData::Module(module) => {
                description.kind = "module";
                module.name_str(vm.interns).clone_into(&mut description.name);
                description.doc = stored_doc(member_dict(object, vm).expect("module has a namespace"), vm);
            }
            HeapData::Class(class) => {
                description.kind = "class";
                class.name().as_str(vm.interns).clone_into(&mut description.name);
                description.doc = stored_doc(class.namespace(), vm);
                let init = class
                    .namespace()
                    .get_by_str("__init__", vm.heap, vm.interns)
                    .map(|v| v.clone_with_heap(vm));
                defer_drop!(init, vm);
                description.signature = match init {
                    Some(init @ Value::Ref(id)) if matches!(vm.heap.get(*id), HeapData::BoundMethod(_)) => {
                        describe(init, false, vm)?.signature
                    }
                    Some(init) => function_description(init, true, vm)?.and_then(|description| description.signature),
                    None => Some("()".to_owned()),
                };
            }
            HeapData::HostClassType(class) => {
                description.kind = "class";
                description.callable = true;
                class.name(vm.interns).clone_into(&mut description.name);
                description.doc = stored_doc(class.attrs(), vm);
            }
            _ => {}
        },
        _ => {}
    }
    Ok(description)
}

fn function_description(object: &Value, bound: bool, vm: &mut VM<'_>) -> RunResult<Option<Description>> {
    let (func_id, defaults, doc) = match object {
        Value::DefFunction(id) => {
            let doc = vm
                .interns
                .get_function(*id)
                .docstring
                .map(|id| vm.interns.get_str(id).to_owned());
            (*id, Vec::new(), doc)
        }
        Value::Ref(id) => {
            let (id, defaults, doc) = match vm.heap.get(*id) {
                HeapData::FunctionDefaults(function) => (function.func_id, function.defaults.as_slice(), &function.doc),
                HeapData::Closure(function) => (function.func_id, function.defaults.as_ref(), &function.doc),
                _ => return Ok(None),
            };
            vm.heap
                .tracker
                .check_allocation(defaults.len().saturating_mul(VALUE_SIZE))?;
            let doc = doc.as_str(vm).map(str::to_owned);
            let defaults = defaults
                .iter()
                .map(|value| value.clone_with_heap(vm.heap))
                .collect::<Vec<_>>();
            (id, defaults, doc)
        }
        _ => return Ok(None),
    };
    defer_drop!(defaults, vm);
    let function = vm.interns.get_function(func_id);
    let name = vm.interns.get_str(function.name.name_id).to_owned();
    let is_async = function.is_async;
    let signature = function.signature.format(defaults, bound, vm)?;
    Ok(Some(Description {
        kind: if is_async { "async function" } else { "function" },
        name,
        signature,
        doc,
        callable: true,
        is_async,
    }))
}

fn stored_doc(dict: &Dict, vm: &VM<'_>) -> Option<String> {
    dict.get_by_str("__doc__", vm.heap, vm.interns)
        .and_then(|doc| doc.as_str(vm))
        .map(str::to_owned)
}

fn member_dict<'v>(object: &Value, vm: &'v VM<'_>) -> Option<&'v Dict> {
    let Value::Ref(id) = object else { return None };
    match vm.heap.get(*id) {
        HeapData::Module(module) => match module.namespace() {
            Some(id) => match vm.heap.get(id) {
                HeapData::Dict(dict) => Some(dict),
                _ => unreachable!("module namespace is a dict"),
            },
            None => Some(module.attrs()),
        },
        HeapData::Class(class) => Some(class.namespace()),
        HeapData::HostClassType(class) => Some(class.attrs()),
        _ => None,
    }
}

/// Snapshot values before formatting defaults, whose repr methods may change the namespace.
fn members(object: &Value, vm: &mut VM<'_>) -> RunResult<Vec<(Value, Value)>> {
    let Some(dict) = member_dict(object, vm) else {
        return Ok(Vec::new());
    };
    vm.heap
        .tracker
        .check_allocation(dict.len().saturating_mul(2 * VALUE_SIZE))?;
    let mut members = DropGuard::new(Vec::with_capacity(dict.len()), vm);
    let (items, vm) = members.as_parts_mut();
    for (index, (key, value)) in member_dict(object, vm)
        .expect("namespace still exists")
        .iter()
        .enumerate()
    {
        vm.heap.tracker.check_time_every(index)?;
        if key
            .as_str(vm)
            .is_some_and(|name| !name.starts_with('_') || matches!(name, "__init__" | "__call__"))
        {
            items.push((key.clone_with_heap(vm.heap), value.clone_with_heap(vm.heap)));
        }
    }
    items.sort_unstable_by(|(a, _), (b, _)| a.as_str(vm).cmp(&b.as_str(vm)));
    Ok(members.into_inner())
}

fn append_description(
    output: &mut String,
    name: &str,
    description: &Description,
    member: bool,
    vm: &VM<'_>,
) -> RunResult<()> {
    let mut builder = StringBuilder::from_existing(mem::take(output), &vm.heap.tracker);
    let indent = if member { "    " } else { "" };
    let doc_indent = if member { "        " } else { "    " };
    builder.push_str(indent)?;
    if description.is_async {
        builder.push_str("async ")?;
    }
    builder.push_str(name)?;
    if let Some(signature) = &description.signature {
        builder.push_str(signature)?;
    }
    builder.push('\n')?;
    if description.callable && description.signature.is_none() {
        builder.push_str(doc_indent)?;
        builder.push_str("Signature unavailable.\n")?;
    }
    if let Some(doc) = &description.doc {
        append_doc(&mut builder, doc, doc_indent, vm)?;
    }
    *output = builder.finish_raw()?;
    Ok(())
}

/// Remove common docstring indentation after expanding tabs, preserving relative indentation.
fn append_doc(builder: &mut StringBuilder<'_>, doc: &str, indent: &str, vm: &VM<'_>) -> RunResult<()> {
    let mut expanded = StringBuilder::new(&vm.heap.tracker);
    let mut column = 0;
    for (index, ch) in doc.chars().enumerate() {
        vm.heap.tracker.check_time_every(index)?;
        if ch == '\t' {
            let width = 8 - column % 8;
            expanded.push_repeated(' ', width)?;
            column += width;
        } else {
            expanded.push(ch)?;
            column = if matches!(ch, '\n' | '\r') { 0 } else { column + 1 };
        }
    }
    let expanded = expanded.finish_raw()?;
    let mut lines = expanded.lines();
    let first = lines.next().unwrap_or("").trim_start();
    let margin = lines
        .clone()
        .filter(|line| !line.trim().is_empty())
        .map(|line| line.chars().take_while(|ch| ch.is_whitespace()).count())
        .min()
        .unwrap_or(0);
    let mut started = false;
    let mut blanks = 0;
    for (index, line) in iter::once(first)
        .chain(lines.map(|line| {
            let offset = line.char_indices().nth(margin).map_or(line.len(), |(offset, _)| offset);
            &line[offset..]
        }))
        .enumerate()
    {
        vm.heap.tracker.check_time_every(index)?;
        if line.trim().is_empty() {
            if started {
                blanks += 1;
            }
            continue;
        }
        builder.push_repeated('\n', blanks)?;
        blanks = 0;
        builder.push_str(indent)?;
        builder.push_str(line)?;
        builder.push('\n')?;
        started = true;
    }
    Ok(())
}
