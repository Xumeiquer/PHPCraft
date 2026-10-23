<?php
namespace Group\Lib {
    define('Group\\Lib\\VERSION', '1.2');
    define('Group\\Lib\\BUILD', 42);

    function helper($value)
    {
        return "lib:" . $value;
    }

    function suffix($value)
    {
        return $value . ":suffix";
    }

    class Box
    {
        public $value = 'box';
    }

    class Unit
    {
        public $value = 'unit';
    }

    class Nested
    {
        public $value = 'nested';
    }
}

namespace Group\App {
    use Group\Lib\{
        Box,
        Unit as Crate,
        function helper as h,
        const VERSION as VER,
    };

    $box = new Box();
    $crate = new Crate();
    var_dump($box->value, $crate->value, h('x'), VER, Box::class, Crate::class);
}

namespace Group\Second {
    use \Group\Lib\{Box as Boxy, function helper, const BUILD};

    $box = new Boxy();
    var_dump($box->value, helper('plain'), BUILD, Boxy::class);
}

namespace {
    use function Group\Lib\{suffix};
    use const Group\Lib\{BUILD as B};

    var_dump(suffix('global'), B);
}
