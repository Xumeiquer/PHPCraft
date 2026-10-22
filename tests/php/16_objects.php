<?php
class Base {
    public $value = 1;
    protected $secret = "s";
    public static $count = 0;
    const KIND = "base";

    public function __construct($value) {
        $this->value = $value;
        self::$count++;
    }

    public function describe() {
        return "Base(" . $this->value . ")";
    }

    public static function make($value) {
        return new static($value);
    }
}

class Child extends Base {
    public function describe() {
        return "Child[" . parent::describe() . "]";
    }
}

$child = Child::make(7);
echo $child->describe(), "|", $child->value, "|", get_class($child), "|";
echo $child instanceof Child ? "child" : "no", "|";
echo $child instanceof Base ? "base" : "no", "|";
echo Child::KIND, "|", Child::$count, "|";

$copy = $child;
$copy->value = 9;
echo $child->value, "|", $child === $copy ? "same" : "different", "|";

$clone = clone $child;
$clone->value = 11;
echo $child->value, "|", $clone->value, "|";
echo $clone === $child ? "same" : "different", "|";

function fail($message) {
    throw new InvalidArgumentException($message, 42);
}

try {
    fail("bad");
} catch (LogicException $e) {
    echo get_class($e), ":", $e->getMessage(), ":", $e->getCode(), "|";
} finally {
    echo "finally|";
}

try {
    try {
        throw new RuntimeException("inner");
    } catch (TypeError $e) {
        echo "wrong";
    }
} catch (Exception $e) {
    echo "outer:", $e->getMessage(), "|";
}

try {
    strlen([]);
} catch (TypeError $e) {
    echo "type:", get_class($e), "|";
}

var_dump($child);
print_r($child);
