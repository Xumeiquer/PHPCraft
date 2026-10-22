<?php
echo sprintf("%d|%05d|%-5d|%+d|%x|%X|%o|%b|%c", 42, 42, 42, 42, 255, 255, 8, 5, 65), PHP_EOL;
echo sprintf("%f|%.2f|%10.2f|%-10.2f|%e|%E|%g|%u|%%", 3.14159, 3.14159, 3.14159, 3.14159, 12345.678, 12345.678, 0.000012345, 42), PHP_EOL;
echo sprintf("%s|%10s|%-10s|%.3s", "abc", "abc", "abc", "abcdef"), PHP_EOL;
printf("%s=%d\n", "answer", 42);

echo number_format(1234567.891), PHP_EOL;
echo number_format(1234567.891, 2), PHP_EOL;
echo number_format(1234567.891, 2, ",", "."), PHP_EOL;
echo number_format(-1234.5, 1), PHP_EOL;

var_dump(abs(-5), abs(5.5), abs(-5.5));
var_dump(ceil(1.2), ceil(-1.2), floor(1.8), floor(-1.8), round(1.5), round(2.5), round(-1.5), round(1.2345, 2));
var_dump(sqrt(16), sqrt(2), pow(2, 10), intdiv(7, 2), intdiv(-7, 2), fmod(7.5, 2));
var_dump(max(1, 2, 3), max([4, 9, 2]), min(1, 2, 3), min([4, 9, 2]), max("apple", "banana"));
var_dump(pi(), M_PI);

$arr = range(1, 5);
var_dump($arr, range(0, 10, 3), range("a", "e", 2));

var_dump(is_int(1), is_integer(1), is_float(1.0), is_string("x"), is_bool(false), is_array([]), is_scalar(null), is_null(null));
var_dump(get_debug_type(1), get_debug_type(1.0), get_debug_type("x"), get_debug_type(true), get_debug_type(null), get_debug_type([]));
var_dump(function_exists("strlen"), function_exists("nope"), is_callable("strlen"));

echo str_pad("hi", 6, "*"), PHP_EOL;
echo str_pad("hi", 7, "-=", STR_PAD_BOTH), PHP_EOL;
