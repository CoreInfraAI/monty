use std::{fmt::Write, sync::Arc};

use crate::{
    args::ArgValues,
    bytecode::{CallResult, VM},
    exception_private::{ExcType, ExcTypeExt, RunResult},
    hash::{HashValue, identity_hash},
    heap::{DropWithContext, HeapObjectRead},
    types::{LazyHeapSet, PyTrait, Type, str::allocate_string},
    value::{EitherStr, Value},
};

/// A host-provided callable identified by its external lookup name.
///
/// The name allocation is shared with the heap's weak external-function cache,
/// which preserves identity while any function with this name remains live.
/// An async one (`is_async`, from a host value marked async) yields calls a
/// supporting worker starts without waiting; it is cached apart from a sync
/// function of the same name, so the two stay distinct objects.
#[derive(Debug, serde::Serialize, serde::Deserialize)]
#[serde(from = "DumpedExtFunction")]
pub(crate) struct ExtFunction {
    #[serde(rename = "n")]
    name: Arc<str>,
    #[serde(rename = "a")]
    is_async: bool,
    #[serde(rename = "d", skip_serializing_if = "Option::is_none")]
    docstring: Option<String>,
}

impl ExtFunction {
    /// Creates an external function with an owned, shareable lookup name.
    pub(crate) fn new(name: &str, is_async: bool) -> Self {
        Self {
            name: Arc::from(name),
            is_async,
            docstring: None,
        }
    }

    /// Returns the function name as text.
    pub(crate) fn as_str(&self) -> &str {
        &self.name
    }

    /// Documentation from the most recent host conversion of this function.
    pub(crate) fn docstring(&self) -> Option<&str> {
        self.docstring.as_deref()
    }

    /// Updates the shared proxy so existing aliases see the host's current documentation.
    pub(crate) fn set_docstring(&mut self, docstring: Option<String>) {
        self.docstring = docstring;
    }

    /// Whether calls start without waiting for the host (see [`CallResult::External`]).
    pub(crate) fn is_async(&self) -> bool {
        self.is_async
    }

    /// Returns the shared name used as the weak-cache key.
    pub(crate) fn cache_key(&self) -> Arc<str> {
        Arc::clone(&self.name)
    }

    /// Clones the function name for an external call suspension.
    pub(crate) fn clone_name(&self) -> EitherStr {
        self.name.to_string().into()
    }
}

/// An [`ExtFunction`] as a dump stores it: a bare name before host functions
/// could be async, or a map whose documentation field is optional.
#[derive(serde::Deserialize)]
#[serde(untagged)]
enum DumpedExtFunction {
    Name(Arc<str>),
    Full {
        n: Arc<str>,
        #[serde(default)]
        a: bool,
        #[serde(default)]
        d: Option<String>,
    },
}

impl From<DumpedExtFunction> for ExtFunction {
    fn from(dumped: DumpedExtFunction) -> Self {
        match dumped {
            DumpedExtFunction::Name(name) => Self {
                name,
                is_async: false,
                docstring: None,
            },
            DumpedExtFunction::Full { n, a, d } => Self {
                name: n,
                is_async: a,
                docstring: d,
            },
        }
    }
}

impl<'h> PyTrait<'h> for HeapObjectRead<'h, ExtFunction> {
    /// Indistinguishable from a sandbox `def` to Python code, which is the
    /// point: a host function is passed around like any other function.
    fn py_type(&self, _: &VM<'h>) -> Type {
        Type::Function
    }

    fn py_len(&self, _: &VM<'h>) -> Option<usize> {
        None
    }

    /// Identity only. Two lookups of the same name share one heap entry (the
    /// weak external-function cache), so `is` already answers this.
    fn py_eq_impl(&self, _: &Value, _: &mut VM<'h>) -> RunResult<Option<bool>> {
        Ok(None)
    }

    fn py_hash(&self, _: &mut VM<'h>) -> RunResult<Option<HashValue>> {
        Ok(Some(identity_hash(self.id())))
    }

    fn py_bool(&self, _: &mut VM<'h>) -> RunResult<bool> {
        Ok(true)
    }

    fn py_repr_fmt(&self, f: &mut impl Write, vm: &mut VM<'h>, _: &mut LazyHeapSet) -> RunResult<()> {
        Ok(write!(f, "<function '{}' external>", self.get(vm.heap).as_str())?)
    }

    fn py_getattr(&self, attr: &EitherStr, vm: &mut VM<'h>) -> RunResult<Option<CallResult>> {
        let function = self.get(vm.heap);
        let value = match attr.as_str(vm.interns) {
            "__name__" => allocate_string(function.as_str(), vm.heap),
            "__doc__" => function
                .docstring()
                .map_or(Value::None, |doc| allocate_string(doc, vm.heap)),
            _ => return Ok(None),
        };
        Ok(Some(CallResult::Value(value)))
    }

    fn py_call_attr(&mut self, vm: &mut VM<'h>, attr: &EitherStr, args: ArgValues) -> RunResult<CallResult> {
        args.drop_with(vm);
        let type_name = match attr.as_str(vm.interns) {
            "__name__" => "str",
            "__doc__" => {
                if self.get(vm.heap).docstring().is_some() {
                    "str"
                } else {
                    "NoneType"
                }
            }
            name => return Err(ExcType::attribute_error("function", name)),
        };
        Err(ExcType::type_error_not_callable_object(type_name))
    }

    /// Suspends: only the host can run this, so the VM yields the name and the
    /// arguments and waits to be resumed with the result. An async function's
    /// call is flagged, so a supporting worker can start it without waiting.
    fn py_call(&mut self, args: ArgValues, vm: &mut VM<'h>) -> RunResult<CallResult> {
        let function = self.get(vm.heap);
        Ok(CallResult::External {
            name: function.clone_name(),
            args,
            is_async: function.is_async(),
        })
    }
}
