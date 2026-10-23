<?php
// Visibility enforcement, __toString() in string coercions, and finally on
// every exit path.

class Base {
    public $pub = "pub";
    protected $prot = "prot";
    private $priv = "priv";
    protected static $sprot = "sprot";
    private static $spriv = "spriv";

    public function inside(): string {
        return $this->prot . $this->priv . self::$sprot . self::$spriv;
    }

    private function privateMethod(): string { return "private"; }
    protected function protectedMethod(): string { return "protected"; }
    private static function privateStatic(): string { return "private-static"; }
    protected static function protectedStatic(): string { return "protected-static"; }
}

class Child extends Base {
    public function inside(): string {
        return $this->prot . self::$sprot . $this->protectedMethod();
    }

    public function readParent(Base $base): string {
        return $base->prot;
    }

    public function readProtectedMethodOn(Base $base): string {
        return $base->protectedMethod();
    }
}

echo (new Base)->inside(), "\n";
echo (new Child)->inside(), "\n";
echo (new Child)->readParent(new Base), "\n";
echo (new Child)->readProtectedMethodOn(new Base), "\n";

// isset()/empty()/?? treat inaccessible properties as unset instead of
// raising a visibility error.
var_dump(isset((new Base)->prot), isset((new Base)->priv));
var_dump(isset(Base::$sprot), isset(Base::$spriv));
var_dump(empty((new Base)->priv), empty(Base::$spriv));
var_dump((new Base)->priv ?? "fallback", Base::$spriv ?? "fallback");

$outside = [
    fn() => (new Base)->prot,
    fn() => (new Base)->priv,
    fn() => Base::$sprot,
    fn() => Base::$spriv,
    fn() => (new Base)->privateMethod(),
    fn() => (new Base)->protectedMethod(),
    fn() => Base::privateStatic(),
    fn() => Base::protectedStatic(),
];
foreach ($outside as $call) {
    try {
        $call();
    } catch (Error $error) {
        echo $error->getMessage(), "\n";
    }
}

// __toString() is used by string-typed builtins and untyped string contexts.
class Stringy {
    public $value;
    public function __construct($value) { $this->value = $value; }
    public function __toString(): string { return "(" . $this->value . ")"; }
}

$stringy = new Stringy("x");
var_dump(strlen($stringy));
var_dump(strtoupper($stringy));
var_dump(trim($stringy));
var_dump(substr($stringy, 0, 2));
var_dump(strpos($stringy, "x"));
var_dump(str_repeat($stringy, 2));
var_dump(str_replace("x", "y", $stringy));
var_dump(implode("-", [$stringy, new Stringy("z")]));
var_dump(sprintf("[%s]", $stringy));
var_dump(strval($stringy));
var_dump(str_contains($stringy, "x"));

class NoString {}

try {
    strlen(new NoString);
} catch (TypeError $error) {
    echo $error->getMessage(), "\n";
}
try {
    strval(new NoString);
} catch (Error $error) {
    echo $error->getMessage(), "\n";
}

// finally runs on return, rethrow, and exceptions thrown from catch.
function returnFromTry(): string {
    try {
        return "try";
    } finally {
        echo "finally-try\n";
    }
}

function returnFromCatch(): string {
    try {
        throw new Exception("first");
    } catch (Exception $error) {
        return "catch";
    } finally {
        echo "finally-catch\n";
    }
}

function nested(): string {
    try {
        try {
            return "nested";
        } finally {
            echo "finally-inner\n";
        }
    } finally {
        echo "finally-outer\n";
    }
}

function override(): string {
    try {
        return "discarded";
    } finally {
        return "finally";
    }
}

function throwFromCatch(): void {
    try {
        throw new Exception("inner");
    } catch (Exception $error) {
        throw new Exception("outer");
    } finally {
        echo "finally-throw\n";
    }
}

function uncaughtFromTry(): void {
    try {
        throw new Exception("uncaught");
    } finally {
        echo "finally-uncaught\n";
    }
}

function throwFromFinally(): string {
    try {
        try {
            return "discarded";
        } finally {
            echo "finally-from-return\n";
            throw new Exception("from-finally");
        }
    } catch (Exception $error) {
        return $error->getMessage();
    } finally {
        echo "finally-outer\n";
    }
}

echo returnFromTry(), "\n";
echo returnFromCatch(), "\n";
echo nested(), "\n";
echo override(), "\n";
try {
    throwFromCatch();
} catch (Exception $error) {
    echo $error->getMessage(), "\n";
}
try {
    uncaughtFromTry();
} catch (Exception $error) {
    echo $error->getMessage(), "\n";
}
echo throwFromFinally(), "\n";
