<?php
namespace One {
    function name() {
        return __NAMESPACE__;
    }

    class Item {
        public $value = 1;
    }
}

namespace Two {
    use One\Item as Thing;

    echo \One\name(), "|";
    $thing = new Thing();
    echo get_class($thing), "|", $thing->value, "|";

    function name() {
        return "Two";
    }

    echo namespace\name(), "|";
}

namespace {
    function global_name() {
        return "global";
    }

    echo \Two\name(), "|", \global_name(), "|";
    echo \One\name(), "|";
}
