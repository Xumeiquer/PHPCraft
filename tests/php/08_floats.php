<?php
$values = [0.0, -0.0, 1.0, 1.5, 0.1, 1.0 / 3.0, 1e10, 1e14, 1e15, 1e-5, 1e-4, 1e-3, 123456789012345.0, 1.7976931348623157e308, 4.9e-324];
foreach ($values as $v) {
    var_dump($v);
    echo $v, PHP_EOL;
}
echo 0.1 + 0.2, PHP_EOL;
var_dump(0.1 + 0.2);
var_dump(0.1 + 0.2 == 0.3);
var_dump(sqrt(-1), NAN);
var_dump((float) PHP_INT_MAX);
var_dump(round(1.005, 2), round(1234.5678, -2), round(1234.5678, 2));
var_dump(floor(-0.5), ceil(-0.5), round(-0.5));
