use crate::defs::{Field, Function, Global, ObjectType, TypeRef};
use crate::env::{EnvBuilder, RuntimePlugin};

/// The Lua 5.3 base library and standard tables available to compiled code.
pub struct LuaStdlib;

impl LuaStdlib {
    pub const NAME: &'static str = "lua-stdlib";
}

fn unary_float(name: &str, doc: &str) -> Function {
    Function::new(name)
        .param("x", TypeRef::Float)
        .returns(TypeRef::Float)
        .doc(doc)
}

fn binary_int(name: &str, doc: &str) -> Function {
    Function::new(name)
        .param("a", TypeRef::Int)
        .param("b", TypeRef::Int)
        .returns(TypeRef::Int)
        .doc(doc)
}

impl RuntimePlugin for LuaStdlib {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn install(&self, env: &mut EnvBuilder) {
        env.add_type(
            ObjectType::library("math")
                .doc("Lua 5.3 `math` library.")
                .field(Field::new("pi", TypeRef::Float).doc("The value of π."))
                .field(Field::new("huge", TypeRef::Float).doc("Positive infinity."))
                .method(
                    Function::new("floor")
                        .param("x", TypeRef::Float)
                        .returns(TypeRef::Int)
                        .doc("Largest integer less than or equal to `x`."),
                )
                .method(
                    Function::new("ceil")
                        .param("x", TypeRef::Float)
                        .returns(TypeRef::Int)
                        .doc("Smallest integer greater than or equal to `x`."),
                )
                .method(unary_float("sqrt", "Square root of `x`."))
                .method(unary_float("abs", "Absolute value of `x`."))
                .method(unary_float("exp", "e raised to the power `x`."))
                .method(unary_float("log", "Natural logarithm of `x`."))
                .method(unary_float("sin", "Sine of `x` (radians)."))
                .method(unary_float("cos", "Cosine of `x` (radians)."))
                .method(unary_float("tan", "Tangent of `x` (radians)."))
                .method(
                    Function::new("fmod")
                        .param("x", TypeRef::Float)
                        .param("y", TypeRef::Float)
                        .returns(TypeRef::Float)
                        .doc("Remainder of `x / y` rounding towards zero."),
                )
                .method(
                    Function::new("min")
                        .variadic()
                        .returns_numeric_promotion()
                        .doc("Smallest argument."),
                )
                .method(
                    Function::new("max")
                        .variadic()
                        .returns_numeric_promotion()
                        .doc("Largest argument."),
                )
                .method(
                    Function::new("random")
                        .optional_param("m", TypeRef::Int)
                        .optional_param("n", TypeRef::Int)
                        .returns(TypeRef::Float)
                        .doc("Pseudo-random number (float in [0,1) or integer in [m, n])."),
                )
                .method(
                    Function::new("randomseed")
                        .param("seed", TypeRef::Int)
                        .doc("Seeds the pseudo-random generator."),
                ),
        );

        env.add_type(
            ObjectType::library("string")
                .doc("Lua 5.3 `string` library.")
                .method(
                    Function::new("char")
                        .variadic()
                        .returns(TypeRef::Str)
                        .doc("String made of the given byte values."),
                )
                .method(
                    Function::new("byte")
                        .param("s", TypeRef::Str)
                        .optional_param("i", TypeRef::Int)
                        .returns(TypeRef::Int)
                        .doc("Byte value of `s[i]`."),
                )
                .method(
                    Function::new("len")
                        .param("s", TypeRef::Str)
                        .returns(TypeRef::Int)
                        .doc("Length of `s` in bytes."),
                )
                .method(
                    Function::new("rep")
                        .param("s", TypeRef::Str)
                        .param("n", TypeRef::Int)
                        .returns(TypeRef::Str)
                        .doc("`s` repeated `n` times."),
                )
                .method(
                    Function::new("format")
                        .param("fmt", TypeRef::Str)
                        .variadic()
                        .returns(TypeRef::Str)
                        .doc("printf-style formatting."),
                )
                .method(
                    Function::new("sub")
                        .param("s", TypeRef::Str)
                        .param("i", TypeRef::Int)
                        .optional_param("j", TypeRef::Int)
                        .returns(TypeRef::Str)
                        .doc("Substring from `i` to `j` (1-based, inclusive)."),
                )
                .method(
                    Function::new("upper")
                        .param("s", TypeRef::Str)
                        .returns(TypeRef::Str)
                        .doc("Upper-cased copy of `s`."),
                )
                .method(
                    Function::new("lower")
                        .param("s", TypeRef::Str)
                        .returns(TypeRef::Str)
                        .doc("Lower-cased copy of `s`."),
                )
                .method(
                    Function::new("find")
                        .param("s", TypeRef::Str)
                        .param("pattern", TypeRef::Str)
                        .returns(TypeRef::Any)
                        .doc("Start index of the first match of `pattern`, or `nil`."),
                )
                .method(
                    Function::new("gsub")
                        .param("s", TypeRef::Str)
                        .param("pattern", TypeRef::Str)
                        .param("repl", TypeRef::Any)
                        .returns(TypeRef::Str)
                        .doc("Copy of `s` with every match of `pattern` replaced."),
                )
                .method(
                    Function::new("match")
                        .param("s", TypeRef::Str)
                        .param("pattern", TypeRef::Str)
                        .returns(TypeRef::Any)
                        .doc("First capture of `pattern` in `s`, or `nil`."),
                )
                .method(
                    Function::new("reverse")
                        .param("s", TypeRef::Str)
                        .returns(TypeRef::Str)
                        .doc("`s` reversed."),
                ),
        );

        env.add_type(
            ObjectType::library("table")
                .doc("Lua 5.3 `table` library.")
                .method(
                    Function::new("insert")
                        .param("t", TypeRef::Any)
                        .param("value", TypeRef::Any)
                        .doc("Appends `value` (or inserts at a position when given 3 arguments)."),
                )
                .method(
                    Function::new("remove")
                        .param("t", TypeRef::Any)
                        .optional_param("pos", TypeRef::Int)
                        .returns(TypeRef::Any)
                        .doc("Removes and returns the element at `pos` (default: last)."),
                )
                .method(
                    Function::new("concat")
                        .param("t", TypeRef::Any)
                        .optional_param("sep", TypeRef::Str)
                        .returns(TypeRef::Str)
                        .doc("Joins the elements of `t` with `sep`."),
                )
                .method(
                    Function::new("sort")
                        .param("t", TypeRef::Any)
                        .optional_param("comp", TypeRef::Any)
                        .doc("Sorts `t` in place."),
                )
                .method(
                    Function::new("unpack")
                        .param("t", TypeRef::Any)
                        .returns(TypeRef::Any)
                        .doc("Returns the elements of `t` as multiple values."),
                ),
        );

        env.add_type(
            ObjectType::library("bit32")
                .doc("Bitwise operations library.")
                .method(binary_int("band", "Bitwise AND."))
                .method(binary_int("bor", "Bitwise OR."))
                .method(binary_int("bxor", "Bitwise XOR."))
                .method(binary_int("lshift", "Logical left shift."))
                .method(binary_int("rshift", "Logical right shift."))
                .method(
                    Function::new("bnot")
                        .param("a", TypeRef::Int)
                        .returns(TypeRef::Int)
                        .doc("Bitwise NOT."),
                ),
        );

        for library in ["math", "string", "table", "bit32"] {
            env.add_global(Global::object(library).doc(format!("Lua `{library}` library.")));
        }

        let functions = [
            Function::new("tonumber")
                .param("value", TypeRef::Any)
                .optional_param("base", TypeRef::Int)
                .returns(TypeRef::Float)
                .doc("Converts `value` to a number, or `nil`."),
            Function::new("tostring")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Str)
                .doc("String representation of `value`."),
            Function::new("type")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Str)
                .doc("Lua type name of `value`."),
            Function::new("ipairs")
                .param("t", TypeRef::Any)
                .returns(TypeRef::Any)
                .doc("Iterator over the array part of `t`."),
            Function::new("pairs")
                .param("t", TypeRef::Any)
                .returns(TypeRef::Any)
                .doc("Iterator over all key/value pairs of `t`."),
            Function::new("next")
                .param("t", TypeRef::Any)
                .optional_param("key", TypeRef::Any)
                .returns(TypeRef::Any)
                .doc("Next key/value pair of `t` after `key`."),
            Function::new("unpack")
                .param("t", TypeRef::Any)
                .returns(TypeRef::Any)
                .doc("Returns the elements of `t` as multiple values."),
            Function::new("pcall")
                .param("f", TypeRef::Any)
                .variadic()
                .returns(TypeRef::Any)
                .doc("Calls `f` in protected mode, returning a success flag."),
            Function::new("error")
                .param("message", TypeRef::Any)
                .optional_param("level", TypeRef::Int)
                .doc("Raises an error."),
            Function::new("assert")
                .param("value", TypeRef::Any)
                .optional_param("message", TypeRef::Str)
                .returns(TypeRef::Any)
                .doc("Raises an error if `value` is false or nil."),
        ];

        for function in functions {
            env.add_global(Global::function(function));
        }
    }
}
