<?php
class DestructureBox
{
    public $items = [1, 2];
    public static $shared = [3, 4];
}

echo "list/array syntax\n";
[$a, $b] = [1, 2];
list($c, $d) = [3, 4];
var_dump($a, $b, $c, $d);

echo "holes and nesting\n";
[, $e, , $f] = [10, 20, 30, 40];
[[$g, $h], $i] = [[5, 6], 7];
list($j, list($k, $l)) = [8, [9, 10]];
var_dump($e, $f, $g, $h, $i, $j, $k, $l);

echo "keyed targets\n";
['x' => $m, 'y' => $n] = ['x' => 11, 'y' => 12];
var_dump($m, $n);
$key = 1;
list($key++ => $o, $key => $p) = [13, 14, 15];
var_dump($key, $o, $p);

echo "assignment value\n";
$result = ([$q, [$r, $s]] = [16, [17, 18]]);
var_dump($result, $q, $r, $s);

echo "references\n";
$source = [19, 20, 21];
[&$t, &$u] = $source;
$t = 190;
$u = 200;
var_dump($source, $t, $u);

$box = new DestructureBox();
[&$v, $w] = $box->items;
$v = 210;
var_dump($box->items, $w);

[&$x1, $x2] = DestructureBox::$shared;
$x1 = 310;
var_dump(DestructureBox::$shared, $x2);

$keyed = ['row' => [22, 23]];
['row' => [&$y1, $y2]] = $keyed;
$y1 = 220;
var_dump($keyed, $y1, $y2);

echo "property and static dimensions\n";
$box->items[0] = 24;
$box->items[] = 25;
unset($box->items[1]);
var_dump($box->items);
DestructureBox::$shared[0] = 33;
DestructureBox::$shared[] = 34;
var_dump(DestructureBox::$shared);
[$box->items[0], $box->items[1]] = [26, 27];
var_dump($box->items);

echo "foreach\n";
foreach ([[28, 29], [30, 31]] as [$z1, $z2]) {
    var_dump($z1, $z2);
}
foreach ([[32, 33], [34, 35]] as list($aa, $bb)) {
    var_dump($aa, $bb);
}
foreach ([['left' => 36, 'right' => 37]] as ['right' => $cc, 'left' => $dd]) {
    var_dump($cc, $dd);
}
foreach (['first' => [38, 39], 'second' => [40, 41]] as $label => [$ee, $ff]) {
    var_dump($label, $ee, $ff);
}

echo "foreach references\n";
$rows = [[42, 43], [44, 45]];
foreach ($rows as [&$gg, $hh]) {
    $gg += 400;
}
var_dump($rows);
$keyedRows = ['a' => [46, 47], 'b' => [48, 49]];
foreach ($keyedRows as $name => [&$ii, $jj]) {
    $ii += 500;
}
var_dump($keyedRows, $name);
$deep = [[[50, 51], 52], [[53, 54], 55]];
foreach ($deep as [[&$kk, $ll], $mm]) {
    $kk += 600;
}
var_dump($deep, $hh, $jj, $mm);
