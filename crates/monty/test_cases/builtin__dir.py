# === Current scope ===
module_value = 42
assert 'module_value' in dir()
assert 'defined_later' not in dir()
assert 'len' not in dir()
defined_later = True
assert dir() == sorted(set(dir()))


def local_scope(parameter, *, keyword=1):
    local = 2
    before = dir()
    after = 3
    assert before == ['keyword', 'local', 'parameter']
    assert dir() == ['after', 'before', 'keyword', 'local', 'parameter']
    assert 'module_value' not in dir()
    return local + after


assert local_scope(0) == 5


def outer(captured):
    def inner(parameter):
        names = dir()
        assert names == ['captured', 'parameter']
        return captured + parameter

    return inner


assert outer(3)(4) == 7


class ClassScope:
    visible = 1
    before = dir()
    later = 2


assert 'visible' in ClassScope.before
assert 'before' not in ClassScope.before
assert 'later' not in ClassScope.before

namespace = {'z': 1, 'a': 2}
assert eval('dir()', {}, namespace) == ['a', 'z']
exec('before = dir(); later = 3', {}, namespace)
assert namespace['before'] == ['a', 'z']
assert eval('dir()', {}, namespace) == ['a', 'before', 'later', 'z']


# === Functions and bound methods ===
def tool(value=1):
    """Tool documentation."""
    return value


async def async_tool(value):
    return value


def annotated(value: MissingType):
    return value


for fn in (tool, async_tool, annotated, outer(1), lambda: None):
    names = dir(fn)
    assert names == sorted(set(names))
    assert {'__name__', '__doc__', '__annotations__', '__annotate__'} <= set(names)
    fn.__doc__ = ['changed']
    assert dir(fn) == names


# === Classes and instances ===
class Tools:
    """A tool collection."""

    category = 'demo'

    def __init__(self):
        self.label = 'instance'

    def run(self, value=1):
        """Run a tool."""
        return value


obj = Tools()
assert {'category', 'run', '__doc__', '__init__'} <= set(dir(Tools))
assert {'category', 'run', '__doc__', '__init__', 'label', '__class__'} <= set(dir(obj))
assert '__name__' not in dir(Tools)
assert 'label' not in dir(Tools)
assert '__doc__' in dir(obj.run)
assert obj.run.__doc__ == 'Run a tool.'
obj.category = 'shadowed'
assert dir(obj).count('category') == 1
Tools.new_method = tool
assert 'new_method' in dir(obj)
names = dir(obj)
names.append('only_in_the_list')
assert 'only_in_the_list' not in dir(obj)

long_name = 'a_long_attribute_' + str(42)
setattr(Tools, long_name, 1)
setattr(obj, long_name, 2)
assert dir(obj).count(long_name) == 1
assert getattr(obj, long_name) == 2


def temporary_names(suffix):
    name = 'temporary_attribute_' + str(suffix)
    return dir(type('Temporary', (), {name: None}))


assert 'temporary_attribute_123' in temporary_names(123)


class Annotations:
    unresolved: MissingType


assert '__annotate_func__' in dir(Annotations)
assert '__annotate_func__' in dir(Annotations())


# === Modules list stored names, including private names ===
import math

names = dir(math)
assert names == sorted(set(names))
assert {'sqrt', 'pi', '__doc__'} <= set(names)
math.__doc__ = 'Updated math documentation'
assert dir(math) == names
assert getattr(math, 'sqrt')(9) == 3


# === Argument errors ===
assert repr(dir) == '<built-in function dir>'
try:
    dir(tool, tool)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'dir expected at most 1 argument, got 2'

try:
    dir(object=tool)
    assert False, 'expected TypeError'
except TypeError as exc:
    assert str(exc) == 'dir() takes no keyword arguments'

try:
    eval('dir()', {}, {long_name: 1, 2: 3})
    assert False, 'expected TypeError'
except TypeError:
    pass
