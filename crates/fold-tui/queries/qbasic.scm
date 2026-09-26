(line_number) @constant.numeric
(integer_literal) @constant.numeric
(float_literal) @constant.numeric
(hex_literal) @constant.numeric
(octal_literal) @constant.numeric
(string_literal) @string
(comment) @comment
(apostrophe_comment) @comment
(metacommand) @keyword.directive
(identifier) @variable
(type_sigil) @punctuation.special
(builtin_function) @function.builtin
(user_function) @function
(label) @label
(sub_definition name: (identifier) @function.definition)
(function_definition name: (identifier) @function.definition)
(type_definition name: (identifier) @type.definition)
(type_name) @type
(parameter) @variable.parameter
