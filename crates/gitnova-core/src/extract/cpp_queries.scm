;; C++ symbol extraction queries for gitnova.
;; V2: Reduced variable capture to fields only; fixed function_definition patterns.

;; ── Classes ──
;; Primary: match via name: field
(class_specifier
  name: (type_identifier) @name) @def.class

;; Fallback: catch any class_specifier whose name field isn't a type_identifier
;; (e.g. template-id, qualified name). Extract name in Rust code via child_name.
(class_specifier) @def.class_fallback

;; ── Structs ──
(struct_specifier
  name: (type_identifier) @name) @def.struct

;; Fallback for structs
(struct_specifier) @def.struct_fallback

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

;; ── Variables (class/struct fields only — not local variables or parameters) ──
(field_declaration
  (field_identifier) @name) @def.variable

;; ── Macros ──
(preproc_def
  (identifier) @name) @def.macro
(preproc_function_def
  (identifier) @name) @def.macro

;; ── Functions (all forms) ──
;; Form A: function_declarator accessed via declarator: field
(function_definition
  declarator: (function_declarator
    [(identifier) (field_identifier)] @name)) @def.function_decl

;; Form C: out-of-line definition ClassName::methodName in .cpp
(function_definition
  declarator: (function_declarator
    declarator: (qualified_identifier) @qname)) @def.function_outline

;; ── Namespaces ──
(namespace_definition
  name: (namespace_identifier) @name) @def.namespace

;; ── Labels ──
(labeled_statement
  (statement_identifier) @name) @def.label
