<?php
namespace Demo\App;

use Demo\Lib\Box as Crate;
use function Demo\Lib\label;
use const Demo\Lib\VERSION;

echo __NAMESPACE__, "|";
echo __FILE__ === __DIR__ . "/17_namespaces_includes.php" ? "file" : "bad", "|";

$result = include __DIR__ . "/fixtures/lib.inc";
echo $result["loaded"] ? "yes" : "no", "|";
echo label("x"), "|", VERSION, "|";

$crate = new Crate(5);
echo $crate->get(), "|", Crate::class, "|";
echo strlen("abcd"), "|";
echo include_once __DIR__ . "/fixtures/lib.inc", "|";
echo include_once __DIR__ . "/fixtures/lib.inc", "|";

function local() {
    return "local";
}

echo namespace\local(), "|";
echo \Demo\Lib\label("y"), "|";
