/**
 * Tree-sitter grammar for VCWL, for editor highlighting and bracket matching. It is
 * deliberately loose: a file is a stream of tokens and bracketed groups, so half-written code
 * never turns into error nodes. The compiler (`vcwl check`, `vcwl lsp`) is the real parser.
 */
module.exports = grammar({
  name: "vcwl",
  extras: ($) => [/\s/, $.comment],
  word: ($) => $.identifier,
  rules: {
    source_file: ($) => repeat($._item),
    _item: ($) =>
      choice($.block, $.parens, $.brackets, $.string, $.dimension, $.color, $.float, $.integer, $.boolean, $.keyword, $.type_identifier, $.identifier, $.operator, ",", ";", ":", "."),
    block: ($) => seq("{", repeat($._item), "}"),
    parens: ($) => seq("(", repeat($._item), ")"),
    brackets: ($) => seq("[", repeat($._item), "]"),
    keyword: (_) =>
      choice("fn", "let", "var", "if", "else", "match", "for", "in", "while", "return", "break", "continue", "struct", "enum", "import", "export", "component", "state", "event", "emit", "on", "style", "when", "bind", "app", "slot", "server", "fetch", "routes", "font", "from", "weight", "key", "capabilities"),
    boolean: (_) => choice("true", "false"),
    comment: (_) => token(choice(seq("//", /[^\n]*/), seq("/*", /[^*]*\*+([^/*][^*]*\*+)*/, "/"))),
    string: ($) => seq('"', repeat(choice($.string_content, $.escape_sequence, $.interpolation)), '"'),
    string_content: (_) => token.immediate(prec(1, /[^"\\{]+/)),
    escape_sequence: (_) => token.immediate(/\\./),
    interpolation: ($) => seq(token.immediate("{"), repeat($._item), "}"),
    dimension: (_) => token(seq(/\d+(\.\d+)?/, choice("px", "rem", "em", "%", "ms", "s", "vw", "vh", "deg", "fr"))),
    color: (_) => /#[0-9a-fA-F]{3,8}/,
    float: (_) => /\d+\.\d+/,
    integer: (_) => /\d+/,
    type_identifier: (_) => /[A-Z][A-Za-z0-9_]*/,
    identifier: (_) => /[a-z_][A-Za-z0-9_-]*/,
    operator: (_) => choice("->", "=>", "==", "!=", "<=", ">=", "&&", "||", "+=", "-=", "*=", "/=", "..=", "..", "+", "-", "*", "/", "%", "=", "<", ">", "!", "?", "|", "&", "@"),
  },
});
