<?php
var_dump(1 == 1.0, 1 === 1.0, "1" == 1, "1" === 1, "abc" == 0, "1" == "01", "10" == "1e1");
var_dump(0 == "", "0" == false, null == false, [] == false, "a" == "A");
var_dump(1 <=> 2, 2 <=> 2, 3 <=> 2, "a" <=> "b");
var_dump(1 < 2, "2" < "10", [1, 2] < [1, 3], [1] == [1], [1] === [1]);

var_dump(true && false, true || false, !true, true and false, false or true, true xor true);
var_dump(5 & 3, 5 | 3, 5 ^ 3, ~5, 1 << 4, -16 >> 2);

echo "a" . "b", " ", 1 . 2.5, " ", "x" . 1 + 2, PHP_EOL;

var_dump("10" + 5, "10.5" + 5, "1e2" + 1, " 12 " + 1, null + 1, true + 1, false + 1);
var_dump("5" * "4", "7" % "3", "2" ** "3");

$v = null;
$v ??= "set";
echo $v, PHP_EOL;
$v ??= "not";
echo $v, PHP_EOL;

$x = 0;
$x += (int) "12";
echo $x, PHP_EOL;

var_dump((int) "42abc", (int) "abc", (int) 3.99, (float) "1.5", (string) 12, (bool) "0", (bool) "a", (array) 5);
var_dump(is_numeric("12"), is_numeric("1.2"), is_numeric("1e3"), is_numeric(" 1"), is_numeric("abc"), is_numeric("0x10"));
var_dump(gettype(null), gettype(true), gettype(1), gettype(1.5), gettype("s"), gettype([]));
var_dump(intval("0x1A", 16), intval("101", 2), intval("12.9"), floatval("1.25abc"), strval(3.0), boolval("0"));
