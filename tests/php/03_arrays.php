<?php
$list = [3, 1, 2];
echo count($list), " ", sizeof($list), PHP_EOL;

$map = [
    "a" => 1,
    "b" => 2,
    5 => "five",
    "5" => "also five",
    true => "one",
    null => "null-key",
    1.7 => "float-key",
];
var_dump($map);

$auto = [];
$auto[] = "zero";
$auto[] = "one";
$auto["x"] = "ex";
$auto[] = "two";
var_dump($auto);
var_dump(array_keys($auto), array_values($auto));

$nested = ["a" => ["b" => [1, 2]], "c" => 3];
var_dump($nested);
print_r($nested);
echo PHP_EOL;

var_dump(array_key_exists("a", $nested), array_key_exists("z", $nested));
var_dump(in_array(2, [1, 2, 3]), in_array("2", [1, 2, 3]), in_array("2", [1, 2, 3], true));
var_dump(array_search(2, [1, 2, 3]), array_search("x", [1, 2, 3]));
var_dump(array_sum([1, 2, 3.5]), array_product([2, 3, 4]));
var_dump(array_reverse([1, 2, 3]), array_reverse(["a" => 1, "b" => 2], true));
var_dump(array_slice([1, 2, 3, 4], 1, 2), array_slice(["a" => 1, 3 => 2, 4 => 3], 1));
var_dump(array_merge([1, 2], [3], ["x" => 4]), array_merge(["x" => 1], ["x" => 2]));
var_dump(array_fill(3, 4, "v"), array_combine(["a", "b"], [1, 2]), array_flip(["a" => 1, "b" => 2]));
var_dump(array_map("strtoupper", ["a", "b"]));

$copy = $list;
$copy[] = 99;
echo count($list), " ", count($copy), PHP_EOL;

foreach (["k1" => "v1", "k2" => "v2"] as $key => $value) {
    echo "$key=$value\n";
}
