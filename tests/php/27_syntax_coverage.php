<?php
declare(strict_types=1);

const BASE = 2;
const DERIVED = BASE * 3;

#[Attribute(Attribute::TARGET_ALL)]
class Marker
{
}

#[Marker]
function named(int $a, int $b = 0, ...$rest): void
{
    echo $a, $b;
    foreach ($rest as $key => $value) {
        echo $key, '=', $value;
    }
    echo "\n";
}

named(b: 3, a: 1, extra: 4);
named(...[5, 6, 7]);

function counter(): int
{
    static $n = 0;
    return ++$n;
}

echo BASE, DERIVED, counter(), counter(), "\n";

$visited = 0;
goto label_skip;
echo 'unreachable';
label_skip:
$visited++;
if ($visited < 2) {
    goto label_skip;
}
echo $visited, "\n";

if (BASE === 2):
    echo 'if';
elseif (BASE === 3):
    echo 'no';
else:
    echo 'no';
endif;
for ($i = 0; $i < 2; $i++):
    echo $i;
endfor;
$j = 0;
while ($j < 2):
    echo $j;
    $j++;
endwhile;
foreach ([1, 2] as $value):
    echo $value;
endforeach;
switch (BASE):
    case 2:
        echo "switch\n";
        break;
    default:
        echo "no\n";
endswitch;

class Gadget
{
    public $value = 1;
    public static $count = 2;
    public const KIND = 'g';

    public function double($number)
    {
        return $number * 2;
    }

    public static function plus($number)
    {
        return $number + 1;
    }
}

$gadget = new Gadget();
$gadgetName = 'gadget';
$property = 'value';
$method = 'double';
$class = 'Gadget';

echo $$gadgetName->$property, "\n";
$gadget->$property = 9;
echo $gadget->value, "\n";
echo $gadget->$method(4), "\n";
$staticMethod = 'plus';
echo $class::$staticMethod(5), "\n";
$count = 'count';
echo $class::$$count, "\n";
$class::$count = 8;
echo Gadget::$count, "\n";
$constant = 'KIND';
echo $class::{$constant}, "\n";
$callable = $gadget->$method(...);
echo $callable(6), "\n";
unset($gadget->$property);
var_dump(isset($gadget->value));

$anonymous = new class (3) {
    public function __construct(private int $value)
    {
    }

    public function get(): int
    {
        return $this->value;
    }
};
echo $anonymous->get(), "\n";

interface Named
{
    public function name(): string;
}

trait First
{
    public function name(): string
    {
        return 'first';
    }

    public function clash(): string
    {
        return 'F';
    }
}

trait Second
{
    public function clash(): string
    {
        return 'S';
    }
}

abstract class Base implements Named
{
    abstract public function id(): int;
}

final class Item extends Base
{
    use First, Second {
        First::clash insteadof Second;
        Second::clash as secondClash;
    }

    public function id(): int
    {
        return 7;
    }
}

$item = new Item();
echo $item->name(), $item->id(), $item->clash(), $item->secondClash(), "\n";

readonly class Point
{
    public int $x;

    public function __construct(int $x)
    {
        $this->x = $x;
    }
}

echo (new Point(4))->x, "\n";

enum Suit: string
{
    case Hearts = 'H';
    case Spades = 'S';
}

echo Suit::Hearts->name, Suit::Hearts->value, Suit::from('S')->name;
echo count(Suit::cases()), "\n";

function numbers()
{
    yield 1;
    yield 2;
    yield 'two' => 2;
}

foreach (numbers() as $key => $value) {
    echo $key, ':', $value, ' ';
}
echo "\n";

function innerGenerator()
{
    yield 3;
    yield 4;
}

function outerGenerator()
{
    yield from innerGenerator();
    yield 5;
}

foreach (outerGenerator() as $value) {
    echo $value;
}
echo "\n";

function earlyReturnGenerator()
{
    if (true) {
        return 1;
    }
    yield 2;
}

foreach (earlyReturnGenerator() as $value) {
    echo $value;
}
echo "empty\n";

$arrowGenerator = fn() => yield 6;
foreach ($arrowGenerator() as $value) {
    echo $value;
}
echo "\n";

print "print\n";
