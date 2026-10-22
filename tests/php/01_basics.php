<?php
echo "Hello, PHPCraft!", PHP_EOL;

$a = 10;
$b = 3;
echo $a + $b, " ", $a - $b, " ", $a * $b, " ", $a / $b, " ", $a % $b, " ", $a ** $b, PHP_EOL;

var_dump(1, 1.0, "1", true, false, null);
var_dump(7 / 2, 6 / 2, -7 % 3, 7 % -3, 2 ** -1);
var_dump(1 + 2 * 3, (1 + 2) * 3, 2 ** 3 ** 2, -2 ** 2, (-2) ** 2);

$n = 5;
$n += 2;
echo $n, PHP_EOL;
$n -= 1;
$n *= 3;
$n /= 2;
echo $n, PHP_EOL;
$n %= 5;
echo $n, PHP_EOL;

$i = 0;
echo $i++, " ", $i, " ", ++$i, " ", $i--, " ", $i, PHP_EOL;

define("GREETING", "hi");
define("ANSWER", 42);
echo GREETING, " ", ANSWER, " ", PHP_VERSION, " ", PHP_INT_SIZE, PHP_EOL;
