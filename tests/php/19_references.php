<?php
namespace RefDemo;

function bump(&$value) {
    $value += 10;
}

function by_value($value) {
    $value = 99;
    return $value;
}

class Counter {
    public $value = 0;
    public static $shared = 0;

    public function add(&$value) {
        $value++;
    }
}

$a = 1;
$b = &$a;
$b = 2;
echo $a, "|", $b, "|";
$a = 3;
echo $b, "|";

$first = 1;
$second = 2;
$snapshot = [$first = &$second];
$second = 9;
echo $snapshot[0], "|", $first, "|", $second, "|";

$n = 1;
bump($n);
echo $n, "|";

$m = 1;
echo by_value($m), "|", $m, "|";

$counter = new Counter();
$value = 1;
$counter->add($value);
echo $value, "|";

$property_source = 1;
$property_alias = 2;
$counter->value = ($property_source = &$property_alias);
$property_alias = 8;
echo $counter->value, "|", $property_source, "|", $property_alias, "|";

$static_source = 1;
$static_alias = 2;
Counter::$shared = ($static_source = &$static_alias);
$static_alias = 12;
echo Counter::$shared, "|", $static_source, "|", $static_alias, "|";

$items = [3, 1, 2];
sort($items);
echo implode(",", $items), "|";
rsort($items);
echo implode(",", $items), "|";

$stack = [];
echo array_push($stack, "a", "b"), "|", implode(",", $stack), "|";
echo array_pop($stack), "|", implode(",", $stack), "|";
echo array_shift($stack), "|";
array_unshift($stack, "z", "y");
echo implode(",", $stack), "|";

$original = [2, 1];
$copy = $original;
sort($original);
echo implode(",", $original), "|", implode(",", $copy), "|";

$base = [1];
$alias = &$base;
$alias[] = 2;
echo count($base), "|";

$x = 1;
$by_ref = function () use (&$x) {
    $x = 7;
};
$by_ref();
echo $x, "|";

$y = 1;
$by_value_capture = function () use ($y) {
    $y = 8;
    return $y;
};
echo $by_value_capture(), "|", $y, "|";

$z = 1;
$arrow = fn() => $z;
$z = 2;
echo $arrow(), "|";

$global_value = 1;
function set_global() {
    global $global_value;
    $global_value = 5;
}
set_global();
echo $global_value, "|";

$global_ref = 1;
function bump_global(&$target) {
    $target = 6;
}
bump_global($global_ref);
echo $global_ref;
