<?php
namespace Demo;

function add($a, $b = 1) { return $a + $b; }

class Greeter {
    public string $prefix;

    public function __construct($prefix) { $this->prefix = $prefix; }

    public function greet($name) { return $this->prefix . $name; }

    public static function shout($text) { return strtoupper($text) . "!"; }
}

class Doubler {
    public function __invoke($value) { return $value * 2; }
}

// First-class callables from userland functions keep their parameters.
$add = add(...);
var_dump($add(2));
var_dump($add(2, 5));
var_dump($add);

// Methods bind $this; static methods do not.
$greeter = new Greeter("Hello, ");
$greet = $greeter->greet(...);
var_dump($greet("world"));
var_dump($greet);

$shout = Greeter::shout(...);
var_dump($shout("hey"));
var_dump($shout);

// Builtins expose their reflected parameter names.
$upper = strtoupper(...);
var_dump($upper("mixed"));
var_dump($upper);

// Wrapping an existing closure returns it unchanged; wrapping a first-class
// callable creates a new Closure handle.
$closure = function ($value) { return $value; };
var_dump($closure(...) === $closure);
$wrapped = $upper(...);
var_dump($wrapped("again"));

// Callable arrays work as values, with is_callable(), direct invocation and
// the callback builtins.
$callable = [$greeter, "greet"];
var_dump(is_callable($callable));
var_dump(call_user_func($callable, "call"));
var_dump(call_user_func_array($callable, ["array"]));
var_dump(array_map($callable, ["one", "two"]));
var_dump($callable("direct"));

$static = [Greeter::class, "shout"];
var_dump(is_callable($static));
var_dump($static("static"));
var_dump(call_user_func($static, "func"));

// Callable strings, including Class::method, and objects with __invoke().
$by_name = "strtoupper";
var_dump($by_name("string"));
$by_static = "Demo\\Greeter::shout";
var_dump($by_static("string"));

$doubler = new Doubler();
var_dump(is_callable($doubler));
var_dump(call_user_func($doubler, 21));
$invoke = $doubler(...);
var_dump($invoke(4));

var_dump(is_callable("strtoupper"));
var_dump(is_callable("Demo\\add"));
var_dump(is_callable("Demo\\Greeter::shout"));

// Invalid callables report false without raising.
var_dump(is_callable([$greeter, "missing"]));
var_dump(is_callable(["Demo\\Greeter", "greet"]));
var_dump(is_callable("Demo\\Greeter::greet"));
var_dump(is_callable([$greeter, "greet", "extra"]));
var_dump(is_callable(["Missing\\Class", "method"]));

print_r($add);
print_r($shout);
