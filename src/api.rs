//! Module that defines the extern API of `Engine`.

use crate::any::{Any, AnyExt, Dynamic};
use crate::engine::{make_getter, make_setter, Engine, FnAny, FnSpec, Map};
use crate::error::ParseError;
use crate::fn_call::FuncArgs;
use crate::fn_register::RegisterFn;
use crate::optimize::{optimize_into_ast, OptimizationLevel};
use crate::parser::{lex, parse, parse_global_expr, Position, AST};
use crate::result::EvalAltResult;
use crate::scope::Scope;

use crate::stdlib::{
    any::{type_name, TypeId},
    boxed::Box,
    collections::HashMap,
    string::{String, ToString},
    vec::Vec,
};
#[cfg(not(feature = "no_std"))]
use crate::stdlib::{fs::File, io::prelude::*, path::PathBuf};

// Define callback function types
#[cfg(feature = "sync")]
pub trait ObjectGetCallback<T, U>: Fn(&mut T) -> U + Send + Sync + 'static {}
#[cfg(feature = "sync")]
impl<F: Fn(&mut T) -> U + Send + Sync + 'static, T, U> ObjectGetCallback<T, U> for F {}

#[cfg(not(feature = "sync"))]
pub trait ObjectGetCallback<T, U>: Fn(&mut T) -> U + 'static {}
#[cfg(not(feature = "sync"))]
impl<F: Fn(&mut T) -> U + 'static, T, U> ObjectGetCallback<T, U> for F {}

#[cfg(feature = "sync")]
pub trait ObjectSetCallback<T, U>: Fn(&mut T, U) + Send + Sync + 'static {}
#[cfg(feature = "sync")]
impl<F: Fn(&mut T, U) + Send + Sync + 'static, T, U> ObjectSetCallback<T, U> for F {}

#[cfg(not(feature = "sync"))]
pub trait ObjectSetCallback<T, U>: Fn(&mut T, U) + 'static {}
#[cfg(not(feature = "sync"))]
impl<F: Fn(&mut T, U) + 'static, T, U> ObjectSetCallback<T, U> for F {}

#[cfg(feature = "sync")]
pub trait IteratorCallback:
    Fn(&Dynamic) -> Box<dyn Iterator<Item = Dynamic>> + Send + Sync + 'static
{
}
#[cfg(feature = "sync")]
impl<F: Fn(&Dynamic) -> Box<dyn Iterator<Item = Dynamic>> + Send + Sync + 'static> IteratorCallback
    for F
{
}

#[cfg(not(feature = "sync"))]
pub trait IteratorCallback: Fn(&Dynamic) -> Box<dyn Iterator<Item = Dynamic>> + 'static {}
#[cfg(not(feature = "sync"))]
impl<F: Fn(&Dynamic) -> Box<dyn Iterator<Item = Dynamic>> + 'static> IteratorCallback for F {}

/// Engine public API
impl<'e> Engine<'e> {
    /// Register a custom function.
    pub(crate) fn register_fn_raw(&mut self, fn_name: &str, args: Vec<TypeId>, f: Box<FnAny>) {
        let spec = FnSpec {
            name: fn_name.to_string().into(),
            args,
        };

        if self.functions.is_none() {
            self.functions = Some(HashMap::new());
        }
        self.functions.as_mut().unwrap().insert(spec, f);
    }

    /// Register a custom type for use with the `Engine`.
    /// The type must implement `Clone`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Debug, Clone, Eq, PartialEq)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                    { TestStruct { field: 1 } }
    ///     fn update(&mut self, offset: i64)   { self.field += offset; }
    /// }
    ///
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Use `register_fn` to register methods on the type.
    /// engine.register_fn("update", TestStruct::update);
    ///
    /// assert_eq!(
    ///     engine.eval::<TestStruct>("let x = new_ts(); x.update(41); x")?,
    ///     TestStruct { field: 42 }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_type<T: Any + Clone>(&mut self) {
        self.register_type_with_name::<T>(type_name::<T>());
    }

    /// Register a custom type for use with the `Engine`, with a pretty-print name
    /// for the `type_of` function. The type must implement `Clone`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self { TestStruct { field: 1 } }
    /// }
    ///
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// assert_eq!(
    ///     engine.eval::<String>("let x = new_ts(); type_of(x)")?,
    ///     "rust_out::TestStruct"
    /// );
    ///
    /// // Register the custom type with a name.
    /// engine.register_type_with_name::<TestStruct>("Hello");
    ///
    /// // Register methods on the type.
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// assert_eq!(
    ///     engine.eval::<String>("let x = new_ts(); type_of(x)")?,
    ///     "Hello"
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_type_with_name<T: Any + Clone>(&mut self, name: &str) {
        if self.type_names.is_none() {
            self.type_names = Some(HashMap::new());
        }

        // Add the pretty-print type name into the map
        self.type_names
            .as_mut()
            .unwrap()
            .insert(type_name::<T>().to_string(), name.to_string());
    }

    /// Register an iterator adapter for a type with the `Engine`.
    /// This is an advanced feature.
    pub fn register_iterator<T: Any, F: IteratorCallback>(&mut self, f: F) {
        if self.type_iterators.is_none() {
            self.type_iterators = Some(HashMap::new());
        }

        self.type_iterators
            .as_mut()
            .unwrap()
            .insert(TypeId::of::<T>(), Box::new(f));
    }

    /// Register a getter function for a member of a registered type with the `Engine`.
    ///
    /// The function signature must start with `&mut self` and not `&self`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                { TestStruct { field: 1 } }
    ///
    ///     // Even a getter must start with `&mut self` and not `&self`.
    ///     fn get_field(&mut self) -> i64  { self.field }
    /// }
    ///
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a getter on a property (notice it doesn't have to be the same name).
    /// engine.register_get("xyz", TestStruct::get_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a.xyz")?, 1);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_get<T, U, F>(&mut self, name: &str, callback: F)
    where
        T: Any + Clone,
        U: Any + Clone,
        F: ObjectGetCallback<T, U>,
    {
        self.register_fn(&make_getter(name), callback);
    }

    /// Register a setter function for a member of a registered type with the `Engine`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Debug, Clone, Eq, PartialEq)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                        { TestStruct { field: 1 } }
    ///     fn set_field(&mut self, new_val: i64)   { self.field = new_val; }
    /// }
    ///
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a setter on a property (notice it doesn't have to be the same name)
    /// engine.register_set("xyz", TestStruct::set_field);
    ///
    /// // Notice that, with a getter, there is no way to get the property value
    /// assert_eq!(
    ///     engine.eval::<TestStruct>("let a = new_ts(); a.xyz = 42; a")?,
    ///     TestStruct { field: 42 }
    /// );
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_set<T, U, F>(&mut self, name: &str, callback: F)
    where
        T: Any + Clone,
        U: Any + Clone,
        F: ObjectSetCallback<T, U>,
    {
        self.register_fn(&make_setter(name), callback);
    }

    /// Shorthand for registering both getter and setter functions
    /// of a registered type with the `Engine`.
    ///
    /// All function signatures must start with `&mut self` and not `&self`.
    ///
    /// # Example
    ///
    /// ```
    /// #[derive(Clone)]
    /// struct TestStruct {
    ///     field: i64
    /// }
    ///
    /// impl TestStruct {
    ///     fn new() -> Self                        { TestStruct { field: 1 } }
    ///     fn get_field(&mut self) -> i64          { self.field }
    ///     // Even a getter must start with `&mut self` and not `&self`.
    ///     fn set_field(&mut self, new_val: i64)   { self.field = new_val; }
    /// }
    ///
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, RegisterFn};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Register the custom type.
    /// engine.register_type::<TestStruct>();
    ///
    /// engine.register_fn("new_ts", TestStruct::new);
    ///
    /// // Register a getter and a setter on a property
    /// // (notice it doesn't have to be the same name)
    /// engine.register_get_set("xyz", TestStruct::get_field, TestStruct::set_field);
    ///
    /// assert_eq!(engine.eval::<i64>("let a = new_ts(); a.xyz = 42; a.xyz")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn register_get_set<T, U, G, S>(&mut self, name: &str, get_fn: G, set_fn: S)
    where
        T: Any + Clone,
        U: Any + Clone,
        G: ObjectGetCallback<T, U>,
        S: ObjectSetCallback<T, U>,
    {
        self.register_get(name, get_fn);
        self.register_set(name, set_fn);
    }

    /// Compile a string into an `AST`, which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("40 + 2")?;
    ///
    /// for _ in 0..42 {
    ///     assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile(&self, script: &str) -> Result<AST, ParseError> {
        self.compile_with_scope(&Scope::new(), script)
    }

    /// Compile a string into an `AST` using own scope, which can be used later for evaluation.
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants
    /// // into function calls and operators.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 42_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that `Full` optimization is on, so constants are folded
    /// // into function calls and operators.
    /// let ast = engine.compile_with_scope(&mut scope,
    ///             "if x > 40 { x } else { 0 }"    // all 'x' are replaced with 42
    /// )?;
    ///
    /// // Normally this would have failed because no scope is passed into the 'eval_ast'
    /// // call and so the variable 'x' does not exist.  Here, it passes because the script
    /// // has been optimized and all references to 'x' are already gone.
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_with_scope(&self, scope: &Scope, script: &str) -> Result<AST, ParseError> {
        self.compile_with_scope_and_optimization_level(scope, script, self.optimization_level)
    }

    /// Compile a string into an `AST` using own scope at a specific optimization level.
    pub(crate) fn compile_with_scope_and_optimization_level(
        &self,
        scope: &Scope,
        script: &str,
        optimization_level: OptimizationLevel,
    ) -> Result<AST, ParseError> {
        let scripts = [script];
        let stream = lex(&scripts);
        parse(&mut stream.peekable(), self, scope, optimization_level)
    }

    /// Read the contents of a file into a string.
    #[cfg(not(feature = "no_std"))]
    fn read_file(path: PathBuf) -> Result<String, EvalAltResult> {
        let mut f = File::open(path.clone())
            .map_err(|err| EvalAltResult::ErrorReadingScriptFile(path.clone(), err))?;

        let mut contents = String::new();

        f.read_to_string(&mut contents)
            .map_err(|err| EvalAltResult::ErrorReadingScriptFile(path.clone(), err))?;

        Ok(contents)
    }

    /// Compile a script file into an `AST`, which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script file to an AST and store it for later evaluation.
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let ast = engine.compile_file("script.rhai".into())?;
    ///
    /// for _ in 0..42 {
    ///     engine.eval_ast::<i64>(&ast)?;
    /// }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    pub fn compile_file(&self, path: PathBuf) -> Result<AST, EvalAltResult> {
        self.compile_file_with_scope(&Scope::new(), path)
    }

    /// Compile a script file into an `AST` using own scope, which can be used later for evaluation.
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 42_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let ast = engine.compile_file_with_scope(&mut scope, "script.rhai".into())?;
    ///
    /// let result = engine.eval_ast::<i64>(&ast)?;
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    pub fn compile_file_with_scope(
        &self,
        scope: &Scope,
        path: PathBuf,
    ) -> Result<AST, EvalAltResult> {
        Self::read_file(path).and_then(|contents| Ok(self.compile_with_scope(scope, &contents)?))
    }

    /// Parse a JSON string into a map.
    ///
    /// Set `has_null` to `true` in order to map `null` values to `()`.
    /// Setting it to `false` will cause a _variable not found_ error during parsing.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, AnyExt};
    ///
    /// let engine = Engine::new();
    ///
    /// let map = engine.parse_json(r#"{"a":123, "b":42, "c":false, "d":null}"#, true)?;
    ///
    /// assert_eq!(map.len(), 4);
    /// assert_eq!(map.get("a").cloned().unwrap().cast::<i64>(), 123);
    /// assert_eq!(map.get("b").cloned().unwrap().cast::<i64>(), 42);
    /// assert_eq!(map.get("c").cloned().unwrap().cast::<bool>(), false);
    /// assert_eq!(map.get("d").cloned().unwrap().cast::<()>(), ());
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_object"))]
    pub fn parse_json(&self, json: &str, has_null: bool) -> Result<Map, EvalAltResult> {
        let mut scope = Scope::new();

        // Trims the JSON string and add a '#' in front
        let scripts = ["#", json.trim()];
        let stream = lex(&scripts);
        let ast = parse_global_expr(
            &mut stream.peekable(),
            self,
            &scope,
            OptimizationLevel::None,
        )?;

        // Handle null - map to ()
        if has_null {
            scope.push_constant("null", ());
        }

        self.eval_ast_with_scope(&mut scope, &ast)
    }

    /// Compile a string containing an expression into an `AST`,
    /// which can be used later for evaluation.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile_expression("40 + 2")?;
    ///
    /// for _ in 0..42 {
    ///     assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_expression(&self, script: &str) -> Result<AST, ParseError> {
        self.compile_expression_with_scope(&Scope::new(), script)
    }

    /// Compile a string containing an expression into an `AST` using own scope,
    /// which can be used later for evaluation.
    ///
    /// The scope is useful for passing constants into the script for optimization
    /// when using `OptimizationLevel::Full`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # #[cfg(not(feature = "no_optimize"))]
    /// # {
    /// use rhai::{Engine, Scope, OptimizationLevel};
    ///
    /// let mut engine = Engine::new();
    ///
    /// // Set optimization level to 'Full' so the Engine can fold constants
    /// // into function calls and operators.
    /// engine.set_optimization_level(OptimizationLevel::Full);
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push_constant("x", 10_i64);   // 'x' is a constant
    ///
    /// // Compile a script to an AST and store it for later evaluation.
    /// // Notice that `Full` optimization is on, so constants are folded
    /// // into function calls and operators.
    /// let ast = engine.compile_expression_with_scope(&mut scope,
    ///             "2 + (x + x) * 2"    // all 'x' are replaced with 10
    /// )?;
    ///
    /// // Normally this would have failed because no scope is passed into the 'eval_ast'
    /// // call and so the variable 'x' does not exist.  Here, it passes because the script
    /// // has been optimized and all references to 'x' are already gone.
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    pub fn compile_expression_with_scope(
        &self,
        scope: &Scope,
        script: &str,
    ) -> Result<AST, ParseError> {
        let scripts = [script];
        let stream = lex(&scripts);
        parse_global_expr(&mut stream.peekable(), self, scope, self.optimization_level)
    }

    /// Evaluate a script file.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let result = engine.eval_file::<i64>("script.rhai".into())?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    pub fn eval_file<T: Any + Clone>(&self, path: PathBuf) -> Result<T, EvalAltResult> {
        Self::read_file(path).and_then(|contents| self.eval::<T>(&contents))
    }

    /// Evaluate a script file with own scope.
    ///
    /// # Example
    ///
    /// ```no_run
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 42_i64);
    ///
    /// // Notice that a PathBuf is required which can easily be constructed from a string.
    /// let result = engine.eval_file_with_scope::<i64>(&mut scope, "script.rhai".into())?;
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_std"))]
    pub fn eval_file_with_scope<T: Any + Clone>(
        &self,
        scope: &mut Scope,
        path: PathBuf,
    ) -> Result<T, EvalAltResult> {
        Self::read_file(path).and_then(|contents| self.eval_with_scope::<T>(scope, &contents))
    }

    /// Evaluate a string.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// assert_eq!(engine.eval::<i64>("40 + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval<T: Any + Clone>(&self, script: &str) -> Result<T, EvalAltResult> {
        self.eval_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// assert_eq!(engine.eval_with_scope::<i64>(&mut scope, "x = x + 2; x")?, 42);
    /// assert_eq!(engine.eval_with_scope::<i64>(&mut scope, "x = x + 2; x")?, 44);
    ///
    /// // The variable in the scope is modified
    /// assert_eq!(scope.get_value::<i64>("x").expect("variable x should exist"), 44);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_with_scope<T: Any + Clone>(
        &self,
        scope: &mut Scope,
        script: &str,
    ) -> Result<T, EvalAltResult> {
        // Since the AST will be thrown away afterwards, don't bother to optimize it
        let ast =
            self.compile_with_scope_and_optimization_level(scope, script, OptimizationLevel::None)?;
        self.eval_ast_with_scope(scope, &ast)
    }

    /// Evaluate a string containing an expression.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// assert_eq!(engine.eval_expression::<i64>("40 + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_expression<T: Any + Clone>(&self, script: &str) -> Result<T, EvalAltResult> {
        self.eval_expression_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string containing an expression with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// assert_eq!(engine.eval_expression_with_scope::<i64>(&mut scope, "x + 2")?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_expression_with_scope<T: Any + Clone>(
        &self,
        scope: &mut Scope,
        script: &str,
    ) -> Result<T, EvalAltResult> {
        let scripts = [script];
        let stream = lex(&scripts);
        // Since the AST will be thrown away afterwards, don't bother to optimize it
        let ast = parse_global_expr(&mut stream.peekable(), self, scope, OptimizationLevel::None)?;
        self.eval_ast_with_scope(scope, &ast)
    }

    /// Evaluate an `AST`.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::Engine;
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("40 + 2")?;
    ///
    /// // Evaluate it
    /// assert_eq!(engine.eval_ast::<i64>(&ast)?, 42);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_ast<T: Any + Clone>(&self, ast: &AST) -> Result<T, EvalAltResult> {
        self.eval_ast_with_scope(&mut Scope::new(), ast)
    }

    /// Evaluate an `AST` with own scope.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("x + 2")?;
    ///
    /// // Create initialized scope
    /// let mut scope = Scope::new();
    /// scope.push("x", 40_i64);
    ///
    /// // Compile a script to an AST and store it for later evaluation
    /// let ast = engine.compile("x = x + 2; x")?;
    ///
    /// // Evaluate it
    /// assert_eq!(engine.eval_ast_with_scope::<i64>(&mut scope, &ast)?, 42);
    /// assert_eq!(engine.eval_ast_with_scope::<i64>(&mut scope, &ast)?, 44);
    ///
    /// // The variable in the scope is modified
    /// assert_eq!(scope.get_value::<i64>("x").expect("variable x should exist"), 44);
    /// # Ok(())
    /// # }
    /// ```
    pub fn eval_ast_with_scope<T: Any + Clone>(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<T, EvalAltResult> {
        self.eval_ast_with_scope_raw(scope, ast)?
            .try_cast::<T>()
            .map_err(|a| {
                EvalAltResult::ErrorMismatchOutputType(
                    self.map_type_name((*a).type_name()).to_string(),
                    Position::none(),
                )
            })
    }

    pub(crate) fn eval_ast_with_scope_raw(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<Dynamic, EvalAltResult> {
        ast.0
            .iter()
            .try_fold(().into_dynamic(), |_, stmt| {
                self.eval_stmt(scope, Some(ast.1.as_ref()), stmt, 0)
            })
            .or_else(|err| match err {
                EvalAltResult::Return(out, _) => Ok(out),
                _ => Err(err),
            })
    }

    /// Evaluate a file, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    #[cfg(not(feature = "no_std"))]
    pub fn consume_file(&self, path: PathBuf) -> Result<(), EvalAltResult> {
        Self::read_file(path).and_then(|contents| self.consume(&contents))
    }

    /// Evaluate a file with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    #[cfg(not(feature = "no_std"))]
    pub fn consume_file_with_scope(
        &self,
        scope: &mut Scope,
        path: PathBuf,
    ) -> Result<(), EvalAltResult> {
        Self::read_file(path).and_then(|contents| self.consume_with_scope(scope, &contents))
    }

    /// Evaluate a string, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume(&self, script: &str) -> Result<(), EvalAltResult> {
        self.consume_with_scope(&mut Scope::new(), script)
    }

    /// Evaluate a string with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_with_scope(&self, scope: &mut Scope, script: &str) -> Result<(), EvalAltResult> {
        let scripts = [script];
        let stream = lex(&scripts);

        // Since the AST will be thrown away afterwards, don't bother to optimize it
        let ast = parse(&mut stream.peekable(), self, scope, OptimizationLevel::None)?;
        self.consume_ast_with_scope(scope, &ast)
    }

    /// Evaluate an AST, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_ast(&self, ast: &AST) -> Result<(), EvalAltResult> {
        self.consume_ast_with_scope(&mut Scope::new(), ast)
    }

    /// Evaluate an `AST` with own scope, but throw away the result and only return error (if any).
    /// Useful for when you don't need the result, but still need to keep track of possible errors.
    pub fn consume_ast_with_scope(
        &self,
        scope: &mut Scope,
        ast: &AST,
    ) -> Result<(), EvalAltResult> {
        ast.0
            .iter()
            .try_fold(().into_dynamic(), |_, stmt| {
                self.eval_stmt(scope, Some(ast.1.as_ref()), stmt, 0)
            })
            .map(|_| ())
            .or_else(|err| match err {
                EvalAltResult::Return(_, _) => Ok(()),
                _ => Err(err),
            })
    }

    /// Call a script function defined in an `AST` with multiple arguments.
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # #[cfg(not(feature = "no_function"))]
    /// # {
    /// use rhai::{Engine, Scope};
    ///
    /// let engine = Engine::new();
    ///
    /// let ast = engine.compile(r"
    ///     fn add(x, y) { len(x) + y + foo }
    ///     fn add1(x)   { len(x) + 1 + foo }
    ///     fn bar()     { foo/2 }
    /// ")?;
    ///
    /// let mut scope = Scope::new();
    /// scope.push("foo", 42_i64);
    ///
    /// // Call the script-defined function
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "add", ( String::from("abc"), 123_i64 ) )?;
    /// assert_eq!(result, 168);
    ///
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "add1", ( String::from("abc"), ) )?;
    /// //                                                         ^^^^^^^^^^^^^^^^^^^^^^^^ tuple of one
    /// assert_eq!(result, 46);
    ///
    /// let result: i64 = engine.call_fn(&mut scope, &ast, "bar", () )?;
    /// assert_eq!(result, 21);
    /// # }
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "no_function"))]
    pub fn call_fn<A: FuncArgs, T: Any + Clone>(
        &self,
        scope: &mut Scope,
        ast: &AST,
        name: &str,
        args: A,
    ) -> Result<T, EvalAltResult> {
        let mut arg_values = args.into_vec();
        let mut args: Vec<_> = arg_values.iter_mut().map(Dynamic::as_mut).collect();
        let fn_lib = Some(ast.1.as_ref());
        let pos = Position::none();

        self.call_fn_raw(Some(scope), fn_lib, name, &mut args, None, pos, 0)?
            .try_cast()
            .map_err(|a| {
                EvalAltResult::ErrorMismatchOutputType(
                    self.map_type_name((*a).type_name()).into(),
                    pos,
                )
            })
    }

    /// Optimize the `AST` with constants defined in an external Scope.
    /// An optimized copy of the `AST` is returned while the original `AST` is consumed.
    ///
    /// Although optimization is performed by default during compilation, sometimes it is necessary to
    /// _re_-optimize an AST. For example, when working with constants that are passed in via an
    /// external scope, it will be more efficient to optimize the `AST` once again to take advantage
    /// of the new constants.
    ///
    /// With this method, it is no longer necessary to recompile a large script. The script `AST` can be
    /// compiled just once. Before evaluation, constants are passed into the `Engine` via an external scope
    /// (i.e. with `scope.push_constant(...)`). Then, the `AST is cloned and the copy re-optimized before running.
    #[cfg(not(feature = "no_optimize"))]
    pub fn optimize_ast(
        &self,
        scope: &Scope,
        ast: AST,
        optimization_level: OptimizationLevel,
    ) -> AST {
        let fn_lib = ast.1.iter().map(|fn_def| fn_def.as_ref().clone()).collect();
        optimize_into_ast(self, scope, ast.0, fn_lib, optimization_level)
    }

    /// Override default action of `print` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # use std::sync::RwLock;
    /// use rhai::Engine;
    ///
    /// let result = RwLock::new(String::from(""));
    /// {
    ///     let mut engine = Engine::new();
    ///
    ///     // Override action of 'print' function
    ///     engine.on_print(|s| result.write().unwrap().push_str(s));
    ///
    ///     engine.consume("print(40 + 2);")?;
    /// }
    /// assert_eq!(*result.read().unwrap(), "42");
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "sync")]
    pub fn on_print(&mut self, callback: impl Fn(&str) + Send + Sync + 'e) {
        self.on_print = Some(Box::new(callback));
    }
    /// Override default action of `print` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # use std::sync::RwLock;
    /// use rhai::Engine;
    ///
    /// let result = RwLock::new(String::from(""));
    /// {
    ///     let mut engine = Engine::new();
    ///
    ///     // Override action of 'print' function
    ///     engine.on_print(|s| result.write().unwrap().push_str(s));
    ///
    ///     engine.consume("print(40 + 2);")?;
    /// }
    /// assert_eq!(*result.read().unwrap(), "42");
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "sync"))]
    pub fn on_print(&mut self, callback: impl Fn(&str) + 'e) {
        self.on_print = Some(Box::new(callback));
    }

    /// Override default action of `debug` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # use std::sync::RwLock;
    /// use rhai::Engine;
    ///
    /// let result = RwLock::new(String::from(""));
    /// {
    ///     let mut engine = Engine::new();
    ///
    ///     // Override action of 'print' function
    ///     engine.on_debug(|s| result.write().unwrap().push_str(s));
    ///
    ///     engine.consume(r#"debug("hello");"#)?;
    /// }
    /// assert_eq!(*result.read().unwrap(), r#""hello""#);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(feature = "sync")]
    pub fn on_debug(&mut self, callback: impl Fn(&str) + Send + Sync + 'e) {
        self.on_debug = Some(Box::new(callback));
    }
    /// Override default action of `debug` (print to stdout using `println!`)
    ///
    /// # Example
    ///
    /// ```
    /// # fn main() -> Result<(), rhai::EvalAltResult> {
    /// # use std::sync::RwLock;
    /// use rhai::Engine;
    ///
    /// let result = RwLock::new(String::from(""));
    /// {
    ///     let mut engine = Engine::new();
    ///
    ///     // Override action of 'print' function
    ///     engine.on_debug(|s| result.write().unwrap().push_str(s));
    ///
    ///     engine.consume(r#"debug("hello");"#)?;
    /// }
    /// assert_eq!(*result.read().unwrap(), r#""hello""#);
    /// # Ok(())
    /// # }
    /// ```
    #[cfg(not(feature = "sync"))]
    pub fn on_debug(&mut self, callback: impl Fn(&str) + 'e) {
        self.on_debug = Some(Box::new(callback));
    }
}
