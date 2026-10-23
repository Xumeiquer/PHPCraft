<?php
function compare_asc(int $a, int $b): int
{
    return $a <=> $b;
}

class SortProbe
{
    public static function compare_desc(int $a, int $b): int
    {
        return $b <=> $a;
    }

    public function compare_desc_instance(int $a, int $b): int
    {
        return $b <=> $a;
    }
}

echo "usort and stable ties\n";
$numbers = [3, 1, 4, 1, 5, 9, 2, 6];
var_dump(usort($numbers, fn (int $a, int $b): int => $a <=> $b));
var_dump($numbers);
$rows = [[2, 'first'], [1, 'low'], [2, 'second'], [1, 'also low']];
usort($rows, fn (array $a, array $b): int => $a[0] <=> $b[0]);
var_dump($rows);

echo "uasort\n";
$assoc = ['c' => 3, 'a' => 1, 'b' => 2, 'd' => 2];
var_dump(uasort($assoc, fn (int $a, int $b): int => $a <=> $b));
var_dump($assoc);

echo "uksort\n";
$byKey = ['bbb' => 1, 'a' => 2, 'cc' => 3, 'dd' => 4];
var_dump(uksort($byKey, function (string $a, string $b): int {
    return strlen($a) <=> strlen($b) ?: strcmp($a, $b);
}));
var_dump($byKey);

echo "callable forms\n";
$values = [3, 1, 2];
usort($values, 'compare_asc');
var_dump($values);
usort($values, [SortProbe::class, 'compare_desc']);
var_dump($values);
$probe = new SortProbe();
usort($values, [$probe, 'compare_desc_instance']);
var_dump($values);
$words = ['10', '2', '1'];
usort($words, 'strcmp');
var_dump($words);

echo "shuffle\n";
$deck = range(1, 12);
var_dump(shuffle($deck), count($deck), array_is_list($deck));
sort($deck);
var_dump($deck);
$keyed = ['a' => 1, 'b' => 2, 'c' => 3, 'd' => 4];
shuffle($keyed);
$keyedValues = $keyed;
sort($keyedValues);
var_dump(count($keyed), array_is_list($keyed), $keyedValues);

echo "array_multisort basics\n";
$primary = [3, 1, 2, 1];
$secondary = ['c', 'a', 'b', 'z'];
var_dump(array_multisort($primary, $secondary));
var_dump($primary, $secondary);

echo "array_multisort keys\n";
$stringKeys = ['x' => 3, 'y' => 1, 'z' => 2];
$otherKeys = ['p' => 'c', 'q' => 'a', 'r' => 'b'];
array_multisort($stringKeys, $otherKeys);
var_dump($stringKeys, $otherKeys);

$numericKeys = [5 => 3, 9 => 1, 2 => 2];
$numericOther = [10 => 'c', 20 => 'a', 30 => 'b'];
array_multisort($numericKeys, $numericOther);
var_dump($numericKeys, $numericOther);

$mixedKeys = ['x' => 3, 0 => 1, 2 => 2];
$mixedOther = ['p' => 'c', 'q' => 'a', 'r' => 'b'];
array_multisort($mixedKeys, $mixedOther);
var_dump($mixedKeys, $mixedOther);

echo "array_multisort order and flags\n";
$left = [3, 1, 2];
$right = [2, 2, 1];
array_multisort($left, SORT_DESC, SORT_NUMERIC, $right, SORT_ASC, SORT_NUMERIC);
var_dump($left, $right);

$letters = ['b', 'A', 'a'];
array_multisort($letters, SORT_ASC, SORT_STRING | SORT_FLAG_CASE);
var_dump($letters);

$order = SORT_DESC;
$flags = SORT_STRING;
$orderVariable = ['b', 'c', 'a'];
array_multisort($orderVariable, $order, $flags);
var_dump($orderVariable);

echo "array_multisort several arrays\n";
$first = [2, 1, 2, 1, 3];
$second = [4, 3, 2, 1, 0];
$third = ['w', 'x', 'y', 'z', 'q'];
array_multisort($first, SORT_ASC, $second, SORT_ASC, $third, SORT_DESC);
var_dump($first, $second, $third);

echo "references survive sorting\n";
$packed = [3, 1, 2];
$packedRef = &$packed[1];
$keyed = ['x' => 30, 'y' => 10, 'z' => 20];
$keyedRef = &$keyed['y'];
array_multisort($packed, $keyed);
var_dump($packed, $keyed, $packedRef, $keyedRef);
$packedRef = 100;
$keyedRef = 200;
var_dump($packed, $keyed);
$compared = [3, 1, 2];
$comparedRef = &$compared[1];
usort($compared, fn (int $a, int $b): int => $a <=> $b);
var_dump($compared, $comparedRef);
$comparedRef = 300;
var_dump($compared);
