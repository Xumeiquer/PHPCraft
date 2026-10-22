<?php
// Closures capture by value and can be invoked directly.
$add = function ($a, $b) { return $a + $b; };
var_dump($add(2, 3));
var_dump((function ($x) { return $x * 2; })(4));

$factor = 3;
$multiply = function ($x) use ($factor) { return $x * $factor; };
var_dump($multiply(4));
$factor = 10;
var_dump($multiply(4));

// Arrow functions capture free variables implicitly, also by value.
$arrow = fn($x) => $x + $factor;
var_dump($arrow(5));
$static = static fn($x) => $x + 1;
var_dump($static(1));
$factor = 100;
var_dump($arrow(5));

// Nested arrows and closures capture from their immediate scope.
$make = fn($n) => fn($x) => $x + $n;
$add_five = $make(5);
var_dump($add_five(10));

$outer = function ($a) {
    $local = 'L';
    return function ($b) use ($a, $local) { return [$a, $b, $local]; };
};
var_dump($outer('A')('B'));

// Default arguments and callbacks.
$defaults = fn($a = 1, $b = 2) => $a + $b;
var_dump($defaults(), $defaults(10));
var_dump(array_map(fn($v) => $v * 2, [1, 2, 3]));

// Captured arrays keep PHP's copy-on-write behaviour.
$captured = [1, 2];
$push = function () use ($captured) {
    $captured[] = 3;
    return $captured;
};
var_dump($push());
var_dump($captured);

// Dynamic calls dispatch to closures and callable strings.
$callable = 'strlen';
var_dump($callable('hello'));
$fns = [fn($x) => $x + 1, fn($x) => $x * 2];
foreach ($fns as $fn) {
    var_dump($fn(3));
}
var_dump(is_callable($add), is_callable('strlen'), is_callable(123));
var_dump(is_object($add), is_object([]), is_object('x'));
var_dump(get_class($add));
var_dump(call_user_func(fn($x) => $x + 1, 4));
var_dump(call_user_func_array('strlen', ['hello']));
var_dump(gettype($add), get_debug_type($add));
$alias = $add;
var_dump($alias === $add, $add === function () {});

// Closure objects expose their source location and parameter metadata.
var_dump($add);
