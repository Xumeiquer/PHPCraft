<?php
namespace Demo\Lib;

define("Demo\\Lib\\VERSION", "1.0");

function label($name) {
    return __NAMESPACE__ . ":" . $name;
}

class Box {
    public $value;

    public function __construct($value) {
        $this->value = $value;
    }

    public function get() {
        return $this->value;
    }
}

echo "lib|";
return ["loaded" => true];
