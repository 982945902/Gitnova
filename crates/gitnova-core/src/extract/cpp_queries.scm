;; C++ symbol extraction queries for gitnova.

;; ── Classes ──
(class_specifier
  name: (type_identifier) @name) @def.class

;; ── Structs ──
(struct_specifier
  name: (type_identifier) @name) @def.struct

;; ── Enums ──
(enum_specifier
  name: (type_identifier) @name) @def.enum
(enumerator
  name: (identifier) @name) @def.enum_value

;; ── Unions ──
(union_specifier
  name: (type_identifier) @name) @def.union

;; ── Typedefs ──
(type_definition
  (type_identifier) @name) @def.typedef
(alias_declaration
  name: (type_identifier) @name) @def.typedef

;; ── Variables ──
(declaration
  (identifier) @name) @def.variable
(pointer_declarator
  (identifier) @name) @def.variable
(reference_declarator
  (identifier) @name) @def.variable
(array_declarator
  declarator: (identifier) @name) @def.variable
(parenthesized_declarator
  (identifier) @name) @def.variable
(field_declaration
  (field_identifier) @name) @def.variable

;; ── Macros ──
(preproc_def
  (identifier) @name) @def.macro
(preproc_function_def
  (identifier) @name) @def.macro

;; ── Functions ──
;; Simple function (identifier is direct named child of function_declarator).
;; Wrap in function_definition for full span coverage.
(function_definition
  declarator: (function_declarator
    (identifier) @name)) @def.function
(function_definition
  declarator: (function_declarator
    (field_identifier) @name)) @def.method

;; Out-of-line definition: ClassName::methodName in .cpp
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier) @qname)) @def.function_outline

;; ── Namespaces ──
(namespace_definition
  name: (namespace_identifier) @name) @def.namespace

;; ── Labels ──
(labeled_statement
  (statement_identifier) @name) @def.label
