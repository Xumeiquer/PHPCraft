<?php
function add($a, $b)
{
    return $a + $b;
}

function greet($name = "world", $punct = "!")
{
    return "hello, " . $name . $punct;
}

function factorial($n)
{
    if ($n <= 1) {
        return 1;
    }
    return $n * factorial($n - 1);
}

$total = 100;
function read_global()
{
    global $total;
    return $total;
}

function write_global()
{
    global $total;
    $total += 5;
}

function classify($n)
{
    if ($n < 0) {
        return "negative";
    } elseif ($n === 0) {
        return "zero";
    }
    return "positive";
}

function swap_names($first, $second)
{
    return [$second, $first];
}

echo add(2, 3), " ", greet(), " ", greet("PHP", "?"), " ", factorial(5), PHP_EOL;
echo read_global(), " ";
write_global();
write_global();
echo read_global(), PHP_EOL;

echo classify(-3), " ", classify(0), " ", classify(3), PHP_EOL;
var_dump(swap_names("a", "b"));

function is_even($n)
{
    return $n % 2 == 0;
}
for ($i = 0; $i < 4; $i++) {
    echo $i, ":", is_even($i) ? "even" : "odd", " ";
}
echo PHP_EOL;

function no_return_branch($x)
{
    if ($x) {
        return "given";
    }
}
var_dump(no_return_branch(1), no_return_branch(0));

function with_array($items, $sep = ",")
{
    return implode($sep, $items);
}
echo with_array([1, 2, 3]), " ", with_array([1, 2, 3], "|"), PHP_EOL;
