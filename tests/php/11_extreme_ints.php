<?php
// Boundary arithmetic that must not wrap, panic or loop forever.

var_dump(explode(",", "a,b", PHP_INT_MIN));
var_dump(range(PHP_INT_MAX - 2, PHP_INT_MAX));
var_dump(range(PHP_INT_MAX, PHP_INT_MAX - 2));
var_dump(substr("abc", 1, PHP_INT_MAX));
var_dump(array_slice([1, 2, 3], 1, PHP_INT_MAX));
var_dump(str_pad("a", -1));
var_dump(str_repeat("", PHP_INT_MAX));
var_dump(array_fill(PHP_INT_MAX, 0, 1));

var_dump(range(0, 3));
var_dump(range(0, 1, 0.1));
var_dump(range(1, 0, 0.1));
var_dump(range("a", "e", 2));
