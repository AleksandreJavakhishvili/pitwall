; Pitwall's own highlight query for tree-sitter-kotlin-ng (the crate ships
; none): keywords, literals, comments and types.
[
  "abstract" "actual" "annotation" "as" "by" "catch" "class" "companion"
  "const" "constructor" "crossinline" "data" "do" "else" "enum" "expect"
  "external" "final" "finally" "for" "fun" "if" "import" "in" "infix"
  "init" "inline" "inner" "interface" "internal" "is" "lateinit"
  "noinline" "object" "open" "operator" "out" "override" "package"
  "private" "protected" "public" "return" "sealed" "suspend" "tailrec"
  "throw" "try" "typealias" "val" "value" "var" "vararg" "when" "where"
  "while"
] @keyword
(this_expression) @variable.builtin
[(line_comment) (block_comment)] @comment
[(string_literal) (multiline_string_literal) (character_literal)] @string
(escape_sequence) @escape
[(number_literal) (float_literal)] @number
(user_type) @type
[ "(" ")" "[" "]" "{" "}" ] @punctuation.bracket
