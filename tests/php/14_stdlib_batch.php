<?php
// String search and counting
var_dump(strstr("abcabc", "bc"));
var_dump(strstr("abcabc", "bc", true));
var_dump(strstr("abc", ""));
var_dump(strstr("abc", "z"));
var_dump(strrchr("abcabc", "bc"));
var_dump(strrchr("abc", "z"));
var_dump(strpbrk("abc", "cb"));
var_dump(strpbrk("abc", "z"));
var_dump(substr_count("aaa", "aa"));
var_dump(substr_count("abcabc", "bc", 1));
var_dump(substr_count("abcabc", "bc", -3));
var_dump(substr_count("abcabc", "bc", 0, 3));
var_dump(substr_count("abcabc", "bc", 0, -1));

// String transformations
var_dump(ucwords("hello world-foo"));
var_dump(ucwords("hello world-foo", " -"));
var_dump(ucwords("hello", ""));
var_dump(nl2br("a\nb\r\nc\r"));
var_dump(nl2br("a\nb", false));

// Array helpers
var_dump(array_key_first([5 => 1, 2 => 3]));
var_dump(array_key_last([5 => 1, 2 => 3]));
var_dump(array_key_first([]));
var_dump(array_key_last([]));
var_dump(array_is_list([1, 2, 3]));
var_dump(array_is_list([1 => 1, 0 => 2]));
var_dump(array_unique([1, "1", 2, 2.0, "2", true, null, false]));
var_dump(array_unique([1, "1", 2], SORT_REGULAR));
var_dump(array_unique([1, "1", 2], SORT_NUMERIC));
var_dump(array_diff([1, "1", 2, 3], [1]));
var_dump(array_intersect([1, "1", 2, 3], [1]));
var_dump(array_diff_key(["a" => 1, "b" => 2], ["b" => 9]));
var_dump(array_intersect_key(["a" => 1, "b" => 2], ["b" => 9]));
var_dump(array_chunk([1, 2, 3, 4, 5], 2));
var_dump(array_chunk(["a" => 1, 5 => 2, 3], 2, true));
var_dump(array_chunk(["a" => 1, 5 => 2, 3], 2));
var_dump(array_pad([1, 2], 4, 0));
var_dump(array_pad([1, 2], -4, 0));
var_dump(array_pad(["a" => 1, 5 => 2], 4, 0));
var_dump(array_pad([5 => 1, 2 => 3], 1, 0));

// Math
var_dump(log(8, 2));
var_dump(log(1));
var_dump(log10(1000));
var_dump(exp(0));
var_dump(sin(0), cos(0), tan(0));
var_dump(asin(1), acos(1), atan(1));
var_dump(atan2(1, 1));
var_dump(hypot(3, 4));
var_dump(deg2rad(180), rad2deg(M_PI));
var_dump(fdiv(1, 0), fdiv(0, 0), fdiv(-1, 0));
var_dump(is_nan(NAN), is_finite(1.0), is_infinite(INF));

// URL encoding
var_dump(urlencode("a b~!*()"));
var_dump(rawurlencode("a b~!*()"));
var_dump(urldecode("a+b%20c%7E"));
var_dump(rawurldecode("a+b%20c%7E"));
var_dump(urldecode("a%2"));
