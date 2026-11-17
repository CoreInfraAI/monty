"""Module documentation."""

assert __doc__ == 'Module documentation.'
__doc__ = 'Updated module'


def module_doc():
    return __doc__


assert module_doc() == 'Updated module'


# === Source metadata ===
def documented(a, /, b=2, *, c=3):
    """Summary with Unicode: café."""
    return a + b + c


assert documented.__name__ == 'documented'
assert documented.__doc__ == 'Summary with Unicode: café.'
assert documented(1) == 6
assert module_doc.__doc__ is None
assert (lambda: 'body').__name__ == '<lambda>'
assert (lambda: 'body').__doc__ is None


def concatenated():
    "one " "two"  # fmt: skip


def later_string():
    pass
    'Not documentation'


def fstring():
    f'Not documentation'


def bytestring():
    b'Not documentation'


assert concatenated.__doc__ == 'one two'
assert later_string.__doc__ is None
assert fstring.__doc__ is None
assert bytestring.__doc__ is None


async def async_tool(value=4):
    """Async documentation."""
    return value


assert async_tool.__name__ == 'async_tool'
assert async_tool.__doc__ == 'Async documentation.'
assert repr(module_doc) == f'<function module_doc at {hex(id(module_doc))}>'
assert repr(documented) == f'<function documented at {hex(id(documented))}>'
assert repr(async_tool) == f'<function async_tool at {hex(id(async_tool))}>'


# === Mutation follows object identity ===
def factory():
    def tool():
        """Original."""
        return 7

    return tool


a = factory()
b = factory()
alias = a
assert a is not b
assert a != b
assert repr(a) != repr(b)
assert repr(a).endswith(f' at {hex(id(a))}>')
assert len({a: 'first', b: 'second'}) == 2
assert alias is a
a.__doc__ = 'Updated'
assert alias.__doc__ == 'Updated'
assert b.__doc__ == 'Original.'
assert a() == 7


def default_factory():
    def tool(value=8):
        """Defaulted."""
        return value

    return tool


def closure_factory(value):
    def tool():
        """Captured."""
        return value

    return tool


for factory_fn in (default_factory, lambda: closure_factory(9)):
    first = factory_fn()
    second = factory_fn()
    original_doc = second.__doc__
    other = first
    first.__doc__ = 'Replaced ' + str(first())
    assert other.__doc__ == first.__doc__
    assert second.__doc__ == original_doc
    assert first != second
    assert repr(first) != repr(second)
    assert repr(first).endswith(f' at {hex(id(first))}>')
    assert first.__name__ == 'tool'
    setattr(first, '__doc__', None)
    assert other.__doc__ is None


# === Existing bound methods read the underlying function ===
class Tool:
    """Tool class."""

    def run(self, value=3):
        """Run the tool."""
        return value


instance = Tool()
bound = instance.run
assert Tool.__doc__ == 'Tool class.'
assert instance.__doc__ == 'Tool class.'
assert Tool.run.__name__ == 'run'
assert bound.__name__ == 'run'
assert bound.__doc__ == 'Run the tool.'
Tool.run.__doc__ = 'New instructions'
assert bound.__doc__ == 'New instructions'
assert instance.run.__doc__ == 'New instructions'
assert bound() == 3
try:
    bound.__doc__ = 'Cannot write through the method'
    assert False, 'expected AttributeError'
except AttributeError:
    pass
assert Tool.run.__doc__ == 'New instructions'


# === __doc__ can hold arbitrary values, including callables and cycles ===
for fn in (a, documented, closure_factory(10), async_tool, Tool.run):
    fn.__doc__ = ['mutable']
    other = fn
    other.__doc__.append('documentation')
    assert fn.__doc__ == ['mutable', 'documentation']
    fn.__doc__ = lambda: 42
    assert fn.__doc__() == 42
    fn.__doc__ = fn
    assert fn.__doc__ is fn
    fn.__doc__ = None
    assert fn.__doc__ is None
    try:
        fn.__doc__()
        assert False, 'expected TypeError'
    except TypeError as exc:
        assert str(exc) == "'NoneType' object is not callable"
    try:
        fn.__name__()
        assert False, 'expected TypeError'
    except TypeError as exc:
        assert str(exc) == "'str' object is not callable"

Tool.run.__doc__ = lambda: 43
assert bound.__doc__() == 43


def make_cycle():
    fn = factory()
    fn.__doc__ = [fn]
    captured = closure_factory(fn)
    captured.__doc__ = captured


for _ in range(20):
    make_cycle()


# === Runtime compilation retains documentation and its own module namespace ===
namespace = {}
exec('"Exec module"\ndef f():\n    "Exec function"\n    return __doc__', namespace)
assert namespace['__doc__'] == 'Exec module'
assert namespace['f'].__doc__ == 'Exec function'
namespace['__doc__'] = 'Changed exec module'
assert namespace['f']() == 'Changed exec module'
namespace['f'].__doc__ = 'Changed exec function'
assert namespace['f'].__doc__ == 'Changed exec function'


# === A wrapper can copy documentation using ordinary assignment ===
def decorate(function):
    def wrapper(value):
        return function(value)

    wrapper.__doc__ = function.__doc__
    return wrapper


@decorate
def wrapped(value):
    """Wrapped documentation."""
    return value + 1


assert wrapped.__doc__ == 'Wrapped documentation.'
assert wrapped(4) == 5
