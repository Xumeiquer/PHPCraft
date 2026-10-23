<?php
$before = get_include_path();
$old = set_include_path(__DIR__ . '/fixtures');

var_dump($old === $before);
var_dump(get_include_path() === __DIR__ . '/fixtures');

$loaded = include 'lib.inc';
var_dump($loaded);
var_dump(Demo\Lib\label('x'), Demo\Lib\VERSION);

set_include_path('/definitely/not/here' . PATH_SEPARATOR . __DIR__ . '/fixtures');
$loaded = include_once 'lib.inc';
var_dump($loaded);

set_include_path($before);
var_dump(get_include_path() === $before);
