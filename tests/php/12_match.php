<?php
function describe($value) {
    return match ($value) {
        1, 2, 3 => "small",
        4 => "four",
        default => "other",
    };
}

foreach ([1, 2, 3, 4, 5, "1", 1.0] as $value) {
    echo describe($value), "\n";
}

echo match (true) {
    false => "no",
    true => "yes",
}, "\n";

// `default` may appear before other arms; it only runs when none match.
echo match (7) {
    default => "fallback",
    7 => "seven",
}, "\n";

// Match is an expression and can be nested.
echo match (match (2) { 2 => "two", default => "other" }) {
    "two" => "nested\n",
    default => "miss\n",
};
