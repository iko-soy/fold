((identifier) @variable.builtin (#any-of? @variable.builtin "END" "DONE") ) @variable.builtin
((identifier) @constant.builtin (#any-of? @constant.builtin "true" "false") ) @constant.builtin.boolean

(gather label: _ @label)
(choice label: _ @label)

(blockComment) @comment
(choiceBullets) @keyword.control
(functionCall) @function
(gatherBullets) @keyword.control
(identifier) @identifier
(includePath) @string
(knotHeader (identifier)) @namespace
(knotHeader) @namespace
(lineComment) @comment
(number) @constant.numeric
(stitchHeader (identifier)) @namespace
(stitchHeader) @namespace
(string) @string
(tag) @tag
(todoComment) @info

["(" ")" "{" "}"] @punctuation
["->" "->" "<-" "else"] @keyword.control
["~"] @keyword.operator
["?" ">" "<" ">=" "<=" "==" "!=" "-" "+" "*" "/" "and" "&&" "or" "||" "!" "not" "=" "+=" "-=" "++" "--"] @operator
["CONST"  "VAR"  "temp"  "LIST"] @keyword.storage
["EXTERNAL" "function"] @keyword.function
["INCLUDE"] @keyword.include
["return"] @keyword.control.return
