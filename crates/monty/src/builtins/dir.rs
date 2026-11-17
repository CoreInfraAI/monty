//! Names available in a Python scope or an object's stored attributes.

use std::mem;

use crate::{
    args::ArgValues,
    bytecode::VM,
    defer_drop,
    exception_private::{ExcType, ExcTypeExt, RunResult},
    heap::{DropGuard, HeapData},
    intern::StaticStrings,
    sorting::sort_values,
    types::{Dict, List, host_class::host_class_type, str::allocate_string},
    value::{VALUE_SIZE, Value},
};

/// Lists names without reading attributes, evaluating annotations, or asking the host for lazy attributes.
pub fn builtin_dir(vm: &mut VM<'_>, args: ArgValues) -> RunResult<Value> {
    let positional = args.into_pos_only("dir", vm.heap)?;
    defer_drop!(positional, vm);
    let mut names = DropGuard::new(Vec::new(), vm);
    let (items, vm) = names.as_parts_mut();
    match positional.as_slice() {
        [] => {
            let locals = vm.locals_dict()?;
            defer_drop!(locals, vm);
            let Value::Ref(id) = locals else {
                unreachable!("locals() returns a dict")
            };
            let HeapData::Dict(dict) = vm.heap.get(*id) else {
                unreachable!("locals() returns a dict")
            };
            add_keys(items, dict, vm)?;
        }
        [object] => object_names(items, object, vm)?,
        _ => return Err(ExcType::type_error_at_most("dir", 1, positional.len())),
    }
    sort_values(items, None, false, vm)?;
    // A class member shadowed by an instance member still appears only once.
    items.dedup_by(|a, b| {
        if a.as_str(vm).is_some_and(|name| Some(name) == b.as_str(vm)) {
            mem::replace(a, Value::None).drop_with(vm);
            true
        } else {
            false
        }
    });
    let (items, vm) = names.into_parts();
    Ok(Value::Ref(vm.heap.allocate(HeapData::List(List::new(items)))))
}

fn object_names(names: &mut Vec<Value>, object: &Value, vm: &VM<'_>) -> RunResult<()> {
    match object {
        Value::DefFunction(_) => add_function_names(names, vm),
        Value::Ref(id) => match vm.heap.get(*id) {
            HeapData::Module(module) => {
                let attrs = match module.namespace() {
                    Some(namespace) => match vm.heap.get(namespace) {
                        HeapData::Dict(dict) => dict,
                        _ => unreachable!("module namespace is a dict"),
                    },
                    None => module.attrs(),
                };
                add_keys(names, attrs, vm)?;
                add_static_names(names, &[StaticStrings::DunderDoc], vm)
            }
            HeapData::FunctionDefaults(_) | HeapData::Closure(_) => add_function_names(names, vm),
            HeapData::ExtFunction(_) => {
                add_static_names(names, &[StaticStrings::DunderDoc, StaticStrings::DunderName], vm)
            }
            HeapData::BoundMethod(_) => add_static_names(names, &[StaticStrings::DunderDoc], vm),
            HeapData::Class(class) => add_keys(names, class.namespace(), vm),
            HeapData::Instance(instance) => {
                add_keys(names, instance.attrs(), vm)?;
                let HeapData::Class(class) = vm.heap.get(instance.class()) else {
                    unreachable!("instance class is a Class")
                };
                add_keys(names, class.namespace(), vm)?;
                reserve_names(names, 1, vm)?;
                names.push(allocate_string("__class__", vm.heap));
                Ok(())
            }
            HeapData::HostClassType(class) => add_keys(names, class.attrs(), vm),
            HeapData::HostClass(instance) => {
                add_keys(names, instance.attrs(), vm)?;
                add_keys(names, host_class_type(vm.heap, instance.class_id()).attrs(), vm)?;
                reserve_names(names, 1, vm)?;
                names.push(allocate_string("__class__", vm.heap));
                Ok(())
            }
            _ => Ok(()),
        },
        _ => Ok(()),
    }
}

fn add_function_names(names: &mut Vec<Value>, vm: &VM<'_>) -> RunResult<()> {
    add_static_names(
        names,
        &[
            StaticStrings::DunderAnnotate,
            StaticStrings::DunderAnnotations,
            StaticStrings::DunderDoc,
            StaticStrings::DunderName,
        ],
        vm,
    )
}

fn add_static_names(names: &mut Vec<Value>, additions: &[StaticStrings], vm: &VM<'_>) -> RunResult<()> {
    reserve_names(names, additions.len(), vm)?;
    names.extend(
        additions
            .iter()
            .map(|&name| Value::InternString(vm.interns.intern_static(name))),
    );
    Ok(())
}

fn add_keys(names: &mut Vec<Value>, dict: &Dict, vm: &VM<'_>) -> RunResult<()> {
    reserve_names(names, dict.len(), vm)?;
    for (index, (key, _)) in dict.iter().enumerate() {
        vm.heap.tracker.check_time_every(index)?;
        names.push(key.clone_with_heap(vm.heap));
    }
    Ok(())
}

fn reserve_names(names: &mut Vec<Value>, additional: usize, vm: &VM<'_>) -> RunResult<()> {
    vm.heap
        .tracker
        .check_allocation(additional.saturating_mul(VALUE_SIZE))?;
    names
        .try_reserve_exact(additional)
        .map_err(|_| ExcType::allocation_too_large())?;
    Ok(())
}
