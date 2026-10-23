<?php

// Array element references and write-through in both directions.
$a = [1, 2, 3];
$x = &$a[0];
$x = 10;
echo "element-write ", $a[0], PHP_EOL;
$a[0] = 11;
echo "element-read ", $x, PHP_EOL;

$nested = [['value' => 1]];
$nestedRef = &$nested[0]['value'];
$nestedRef = 5;
echo "nested-write ", $nested[0]['value'], PHP_EOL;
$nested[0]['value'] = 6;
echo "nested-read ", $nestedRef, PHP_EOL;

$alias = 1;
$targets = [];
$targets[] = &$alias;
$targets[0] = 2;
echo "target-append ", $alias, PHP_EOL;
$alias = 3;
echo "target-read ", $targets[0], PHP_EOL;

// References to object and static properties.
class RefBox
{
    public $value = 1;
    public static $shared = 2;

    public function &getValue()
    {
        return $this->value;
    }

    public static function &getShared()
    {
        return self::$shared;
    }
}

$box = new RefBox();
$propertyRef = &$box->value;
$propertyRef = 20;
echo "property-write ", $box->value, PHP_EOL;
$box->value = 21;
echo "property-read ", $propertyRef, PHP_EOL;

$staticRef = &RefBox::$shared;
$staticRef = 30;
echo "static-write ", RefBox::$shared, PHP_EOL;
RefBox::$shared = 31;
echo "static-read ", $staticRef, PHP_EOL;

$propertyTarget = 40;
$box->value = &$propertyTarget;
$box->value = 41;
echo "property-target ", $propertyTarget, PHP_EOL;
$propertyTarget = 42;
echo "property-target-read ", $box->value, PHP_EOL;

$staticTarget = 50;
RefBox::$shared = &$staticTarget;
RefBox::$shared = 51;
echo "static-target ", $staticTarget, PHP_EOL;
$staticTarget = 52;
echo "static-target-read ", RefBox::$shared, PHP_EOL;

// By-reference returns from functions, methods and closures.
function &functionRef(array &$values)
{
    return $values[1];
}

$values = [1, 2, 3];
$functionRef = &functionRef($values);
$functionRef = 60;
echo "function-return ", $values[1], PHP_EOL;

$methodRef = &$box->getValue();
$methodRef = 61;
echo "method-return ", $box->value, PHP_EOL;

$staticMethodRef = &RefBox::getShared();
$staticMethodRef = 62;
echo "static-method-return ", RefBox::$shared, PHP_EOL;

$closure = function &(&$value) {
    return $value;
};
$closureTarget = 70;
$closureRef = &$closure($closureTarget);
$closureRef = 71;
echo "closure-return ", $closureTarget, PHP_EOL;

// By-reference foreach over arrays and objects.
$loop = [1, 2, 3];
foreach ($loop as &$item) {
    $item *= 2;
}
unset($item);
echo "foreach-array ", implode(",", $loop), PHP_EOL;

$loop = ['a' => 1, 'b' => 2];
foreach ($loop as $key => &$item) {
    $item += 10;
}
unset($item);
echo "foreach-keys ", $loop['a'], " ", $loop['b'], PHP_EOL;

$objectLoop = new RefBox();
$objectLoop->value = 3;
foreach ($objectLoop as &$item) {
    $item += 4;
}
unset($item);
echo "foreach-object ", $objectLoop->value, PHP_EOL;

// The last foreach value remains aliased until it is unset.
$last = [1, 2];
foreach ($last as &$item) {
    $item += 1;
}
$last[0] = 90;
echo "foreach-last ", $item, PHP_EOL;
unset($item);
$last[1] = 91;
var_dump(isset($item));

// unset() on variables, elements, properties and static properties.
$one = 1;
$two = &$one;
unset($one);
$two = 2;
var_dump(isset($one), $two);

$removed = [1, 2, 3];
$removedRef = &$removed[1];
unset($removed[1]);
echo "unset-element ", count($removed), " ", $removedRef, PHP_EOL;
$removed[1] = 9;
echo "unset-element-readd ", $removedRef, " ", implode(",", $removed), PHP_EOL;

unset($box->value);
var_dump(isset($box->value));

// A reference inside an array is marked by var_dump.
$marked = [];
$markedValue = null;
$marked[] = &$markedValue;
var_dump($marked);

// By-reference variadic parameters and by-reference arguments to functions.
function variadicRefs(&...$items)
{
    $items[0] = 80;
    $items[1][0] = 81;
}

$variadicScalar = 1;
$variadicArray = [2];
variadicRefs($variadicScalar, $variadicArray);
echo "variadic ", $variadicScalar, " ", $variadicArray[0], PHP_EOL;

function assignElement(&$target, $value)
{
    $target = $value;
}

$elementArgument = [1];
assignElement($elementArgument[0], 82);
echo "element-argument ", $elementArgument[0], PHP_EOL;

$matrix = [[3, 1, 2]];
sort($matrix[0]);
echo "sort-element ", implode(",", $matrix[0]), PHP_EOL;

$sortProperty = new RefBox();
$sortProperty->value = [3, 1, 2];
sort($sortProperty->value);
echo "sort-property ", implode(",", $sortProperty->value), PHP_EOL;

// Sorting flags and key-preserving sorts.
$numbers = ["10", 2, "1"];
sort($numbers, SORT_NUMERIC);
echo "sort-numeric ", implode(",", $numbers), PHP_EOL;

$strings = ["b", "A", "a", "B"];
sort($strings, SORT_STRING | SORT_FLAG_CASE);
echo "sort-case ", implode(",", $strings), PHP_EOL;

$descending = [2, 10, 1];
rsort($descending, SORT_NUMERIC);
echo "rsort-numeric ", implode(",", $descending), PHP_EOL;

$assoc = ["b" => 2, "a" => 3, "c" => 1];
asort($assoc);
foreach ($assoc as $key => $value) {
    echo "asort ", $key, "=", $value, PHP_EOL;
}
arsort($assoc);
foreach ($assoc as $key => $value) {
    echo "arsort ", $key, "=", $value, PHP_EOL;
}

$keyed = ["b" => 2, "a" => 3, "c" => 1];
ksort($keyed);
foreach ($keyed as $key => $value) {
    echo "ksort ", $key, "=", $value, PHP_EOL;
}
krsort($keyed);
foreach ($keyed as $key => $value) {
    echo "krsort ", $key, "=", $value, PHP_EOL;
}
