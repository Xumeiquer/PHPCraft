<?php
$x = 7;
if ($x > 10) {
    echo "big\n";
} elseif ($x > 5) {
    echo "medium\n";
} else {
    echo "small\n";
}

$i = 0;
while ($i < 3) {
    echo "w$i ";
    $i++;
}
echo PHP_EOL;

do {
    echo "d$i ";
    $i--;
} while ($i > 0);
echo PHP_EOL;

for ($j = 0; $j < 5; $j++) {
    if ($j == 1) {
        continue;
    }
    if ($j == 4) {
        break;
    }
    echo "f$j ";
}
echo PHP_EOL;

for ($a = 0, $b = 10; $a < $b; $a += 3, $b -= 3) {
    echo "$a:$b ";
}
echo PHP_EOL;

foreach ([10, 20, 30] as $value) {
    echo "v$value ";
}
echo PHP_EOL;

foreach (["a" => 1, "b" => 2] as $k => $v) {
    echo "$k=$v ";
}
echo PHP_EOL;

$nested = [[1, 2], [3, 4]];
foreach ($nested as $row) {
    foreach ($row as $cell) {
        echo $cell;
    }
    echo "|";
}
echo PHP_EOL;

switch ($x) {
    case 1:
        echo "one\n";
        break;
    case 7:
        echo "seven\n";
    case 8:
        echo "fallthrough\n";
        break;
    default:
        echo "default\n";
}
echo PHP_EOL;

switch ("no-match") {
    case "a":
        echo "a\n";
        break;
    default:
        echo "default only\n";
}

echo true ? "yes" : "no", " ", false ?: "fallback", " ", null ?? "null-coalesce", " ", 0 ?: 5, PHP_EOL;

$val = null;
var_dump(isset($val), isset($undefined), empty($val), empty($undefined), empty(0), empty("0"), empty("a"), empty([]), empty([0]));
$val = 0;
var_dump(isset($val), empty($val));
