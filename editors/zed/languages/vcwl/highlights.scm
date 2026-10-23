(comment) @comment
(string) @string
(escape_sequence) @string.escape
(interpolation "{" @punctuation.special "}" @punctuation.special)
(dimension) @number
(integer) @number
(float) @number
(color) @constant
(boolean) @boolean
(keyword) @keyword
(type_identifier) @type
(operator) @operator

; Contextual keywords (plain names to the loose grammar).
((identifier) @keyword (#any-of? @keyword "impl" "interface" "await" "as" "js" "meta"))
((identifier) @variable.builtin (#eq? @variable.builtin "self"))
((type_identifier) @type.builtin (#eq? @type.builtin "Self"))

; `fn name`, `component Name`, `style Name`, `event name`, `state name`
((keyword) @_k . (identifier) @function (#eq? @_k "fn"))
((keyword) @_k . (identifier) @variable.special (#any-of? @_k "state" "event"))

; Calls and elements: a name right before parentheses or a block.
((identifier) @function . (parens))
((identifier) @tag . (block))
((type_identifier) @constructor . (parens))

; Named arguments and style properties.
((identifier) @property . ":")

["(" ")" "[" "]" "{" "}"] @punctuation.bracket
["," ";" "."] @punctuation.delimiter
