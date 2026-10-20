use crate::defs::{Function, Global, Lowering, TypeRef};
use crate::env::{EnvBuilder, RuntimePlugin};

/// The Python builtins QSysPy understands, and how each one lowers to Lua.
pub struct PythonCore;

impl PythonCore {
    pub const NAME: &'static str = "python-core";
}

fn qualified(module: &str, name: &str) -> Lowering {
    Lowering::Qualified {
        module: module.to_string(),
        name: name.to_string(),
    }
}

impl RuntimePlugin for PythonCore {
    fn name(&self) -> &'static str {
        Self::NAME
    }

    fn install(&self, env: &mut EnvBuilder) {
        let builtins = [
            Function::new("print")
                .variadic()
                .doc("Writes values to the debug output (Lua `print`)."),
            Function::new("len")
                .param("obj", TypeRef::Any)
                .returns(TypeRef::Int)
                .lower_to(Lowering::LengthOperator)
                .doc("Length of a string, list or table (Lua `#obj`)."),
            Function::new("str")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Str)
                .lower_to(Lowering::Rename("tostring".to_string()))
                .doc("String representation of a value (Lua `tostring`)."),
            Function::new("int")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Int)
                .lower_to(Lowering::Rename("tonumber".to_string()))
                .doc("Converts a value to a number (Lua `tonumber`)."),
            Function::new("float")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Float)
                .lower_to(Lowering::Rename("tonumber".to_string()))
                .doc("Converts a value to a number (Lua `tonumber`)."),
            Function::new("bool")
                .param("value", TypeRef::Any)
                .returns(TypeRef::Bool)
                .doc("Converts a value to a boolean."),
            Function::new("range")
                .param("start_or_stop", TypeRef::Int)
                .optional_param("stop", TypeRef::Int)
                .optional_param("step", TypeRef::Int)
                .returns(TypeRef::list(TypeRef::Int))
                .doc("Sequence of integers, used in `for` loops."),
            Function::new("abs")
                .param("x", TypeRef::Float)
                .returns_numeric_promotion()
                .lower_to(qualified("math", "abs"))
                .doc("Absolute value (Lua `math.abs`)."),
            Function::new("min")
                .variadic()
                .returns_numeric_promotion()
                .lower_to(qualified("math", "min"))
                .doc("Smallest of the arguments (Lua `math.min`)."),
            Function::new("max")
                .variadic()
                .returns_numeric_promotion()
                .lower_to(qualified("math", "max"))
                .doc("Largest of the arguments (Lua `math.max`)."),
            Function::new("chr")
                .param("code", TypeRef::Int)
                .returns(TypeRef::Str)
                .lower_to(qualified("string", "char"))
                .doc("Character for a byte value (Lua `string.char`)."),
            Function::new("ord")
                .param("char", TypeRef::Str)
                .returns(TypeRef::Int)
                .lower_to(qualified("string", "byte"))
                .doc("Byte value of a character (Lua `string.byte`)."),
            Function::new("super")
                .returns(TypeRef::Any)
                .doc("Proxy to the base class, for `super().method(...)` calls."),
        ];

        for builtin in builtins {
            env.add_global(Global::function(builtin));
        }

        let dynamic = "dynamic code execution cannot be compiled to Lua ahead of time";
        env.remove("eval", dynamic);
        env.remove("exec", dynamic);
        env.remove("compile", dynamic);

        let introspection = "runtime scope introspection is not supported";
        env.remove("globals", introspection);
        env.remove("locals", introspection);
        env.remove("vars", introspection);

        env.remove(
            "__import__",
            "dynamic imports are not supported; use a top-level `import` statement",
        );
    }
}
