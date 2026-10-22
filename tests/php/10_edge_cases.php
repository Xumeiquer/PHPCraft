<?php
$a = $b = $c = 5;
var_dump($a, $b, $c);
$x = 0;
$y = ($x = 3) + 1;
var_dump($x, $y);

$arr = [1, 2];
$arr[0] += 10;
$arr[1] *= 3;
var_dump($arr);

$nested = [];
$nested["a"]["b"][] = 7;
$nested["a"]["c"] = 8;
var_dump($nested);

$negative = [-5 => "a"];
$negative[] = "b";
var_dump($negative);

$original = ["x" => ["y" => 1]];
$copy = $original;
$copy["x"]["y"] = 2;
var_dump($original, $copy);

switch ("1") {
    case 1:
        echo "loose\n";
        break;
    default:
        echo "strict\n";
}

$s = "a";
$s++;
echo $s, " ";
$s = "Az";
$s++;
echo $s, " ";
$s = "z";
$s++;
echo $s, PHP_EOL;

var_dump(PHP_INT_MAX + 1, PHP_INT_MIN - 1, PHP_INT_MAX * 2);
var_dump([1, 2] === [1, 2], ["a" => 1] == ["a" => "1"], [1, 2] <=> [1, 3]);

$undefined = [];
var_dump($undefined["x"]["y"] ?? "default", isset($undefined["x"]["y"]), empty($undefined["x"]));
var_dump($undefined["x"] ?? null);

$text = "abc";
var_dump($text[0], $text[1], $text[-1], $text[3] ?? "none");
var_dump(strlen("é"), "é"[0] === "\xc3");

var_dump(trim("xxhixx", "x"), rtrim("abc\n", "\n"), ltrim("0012", "0"));
var_dump(substr("abcdef", 1, -2), substr("abcdef", -3, -1), substr("abcdef", 10));
var_dump(strpos("abcabc", "a", 1), strrpos("abcabc", "b"));

var_dump(range("a", "e", 2), range(5, 1), range(0, 1, 0.25));

echo number_format(0.5), "|", number_format(-0.5, 0), "|", number_format(1234.5678, 3, ".", ""), PHP_EOL;

var_dump(array_slice([1, 2, 3], 1, null), array_slice([1, 2, 3], -2, 1));
var_dump(array_fill(-2, 4, "x"), array_fill(0, 0, "x"));
