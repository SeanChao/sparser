(
  (comment)+ @comment
  .
  (function_definition
    function_name: ((identifier) @name)
    body: (
      (function_body) @func_body
    )
  ) @func_src
)
