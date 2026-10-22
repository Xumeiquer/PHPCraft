<?php
$name = "World";
$n = 3;
echo "Hello, $name! {$n} times\n";
echo "Single: $name[0] / {$name[0]}\n";

$arr = ["key" => "value", 7 => "seven"];
echo "Interp: {$arr["key"]} {$arr[7]}\n";

echo 'No $interpolation {$name}\n';
echo "Escapes: \t|\\|\"|\$name|\x41|\101|\u{1F600}\n";

$heredoc = <<<EOT
  line one $name
  line two {$n}
  EOT;
echo $heredoc, "\n";

$nowdoc = <<<'EOT'
  raw $name
  EOT;
echo $nowdoc, "\n";

echo strtoupper("abc"), " ", strtolower("ABC"), " ", ucfirst("abc"), " ", lcfirst("ABC"), PHP_EOL;
echo strlen("hello"), " ", strrev("hello"), " ", str_repeat("ab", 3), PHP_EOL;
var_dump(substr("abcdef", 1, 3), substr("abcdef", -2), substr("abcdef", 2));
var_dump(strpos("hello", "ll"), strpos("hello", "z"));
var_dump(str_contains("hello", "ell"), str_starts_with("hello", "he"), str_ends_with("hello", "lo"));
var_dump(trim("  x  "), ltrim("  x  "), rtrim("  x  "));
echo str_pad("5", 3, "0", STR_PAD_LEFT), "|", str_pad("ab", 5, "-", STR_PAD_BOTH), "|", str_pad("ab", 4), "|", PHP_EOL;
var_dump(strcmp("a", "b"), chr(65), ord("A"), ord(""));
echo str_replace(["a", "b"], ["x", "y"], "aabb"), " ", implode(",", ["a", "b", "c"]), " ", join("-", [1, 2]), PHP_EOL;
var_dump(explode(",", "a,b,c"), explode(",", "a,b,c", 2), explode(",", "a,b,c", -1));
