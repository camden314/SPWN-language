//! Tools for compiling SPWN into GD object strings
use crate::ast;
use crate::builtin::*;
use crate::levelstring::*;
use std::collections::HashMap;

use crate::parser::{ParseNotes, SyntaxError};
use std::fs;
use std::path::PathBuf;

use crate::compiler_types::*;

#[derive(Debug)]
pub enum RuntimeError {
    UndefinedErr {
        undefined: String,
        desc: String,
        info: CompilerInfo,
    },

    PackageSyntaxError {
        err: SyntaxError,
        info: CompilerInfo,
    },

    IDError {
        id_class: ast::IDClass,
        info: CompilerInfo,
    },

    TypeError {
        expected: String,
        found: String,
        info: CompilerInfo,
    },

    RuntimeError {
        message: String,
        info: CompilerInfo,
    },

    BuiltinError {
        message: String,
        info: CompilerInfo,
    },
}

impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        //write!(f, "SuperErrorSideKick is here!")
        //let mut message = String::from("Runtime/compile error:");
        match self {
            RuntimeError::UndefinedErr {
                undefined,
                desc,
                info,
            } => write!(
                f,
                "{} '{}' is not defined at line {}, pos {}",
                desc, undefined, info.line.0, info.line.1
            ),
            RuntimeError::PackageSyntaxError { err, info } => write!(
                f,
                "Error when parsing library at line {}, pos {}: {}",
                info.line.0, info.line.1, err
            ),
            RuntimeError::IDError { id_class, info } => write!(
                f,
                "Ran out of {} at line {}, pos {}",
                match id_class {
                    ast::IDClass::Group => "groups",
                    ast::IDClass::Color => "colors",
                    ast::IDClass::Item => "item IDs",
                    ast::IDClass::Block => "collision block IDs",
                },
                info.line.0,
                info.line.1
            ),

            RuntimeError::TypeError {
                expected,
                found,
                info,
            } => write!(
                f,
                "Type mismatch: expected {}, found {} (line {}, pos {})",
                expected, found, info.line.0, info.line.1
            ),

            RuntimeError::RuntimeError { message, info } => {
                write!(f, "{} (line {}, pos {})", message, info.line.0, info.line.1)
            }

            RuntimeError::BuiltinError { message, info } => write!(
                f,
                "Error when calling built-in-function: {} (line {}, pos {})",
                message, info.line.0, info.line.1
            ),
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        None
    }
}

pub const NULL_STORAGE: usize = 1;
pub const BUILTIN_STORAGE: usize = 0;

pub fn compile_spwn(
    statements: Vec<ast::Statement>,
    path: PathBuf,
    gd_path: Option<PathBuf>,
    notes: ParseNotes,
) -> Result<Globals, RuntimeError> {
    //variables that get changed throughout the compiling
    let mut globals = Globals::new(notes, path);
    let start_context = Context::new();
    //store at pos 0
    // store_value(Value::Builtins, 1, &mut globals, &start_context);
    // store_value(Value::Null, 1, &mut globals, &start_context);

    let start_info = CompilerInfo {
        depth: 0,
        path: vec!["main scope".to_string()],
        line: statements[0].line,
        func_id: 0,
    };
    use std::time::Instant;

    //println!("Importing standard library...");

    println!(
        "
Building script...
———————————————————————————
"
    );
    let start_time = Instant::now();

    compile_scope(&statements, vec![start_context], &mut globals, start_info)?;

    println!(
        "
———————————————————————————
Built in {} milliseconds!
",
        start_time.elapsed().as_millis()
    );

    Ok(globals)
}

pub fn compile_scope(
    statements: &Vec<ast::Statement>,
    mut contexts: Vec<Context>,
    globals: &mut Globals,
    mut info: CompilerInfo,
) -> Result<(Vec<Context>, Returns), RuntimeError> {
    let mut statements_iter = statements.iter();

    let mut returns: Returns = Vec::new();

    globals.stored_values.increment_lifetimes();

    while let Some(statement) = statements_iter.next() {
        //find out what kind of statement this is
        //let start_time = Instant::now();

        /*println!(
            "{} -> Compiling a statement in {} contexts",
            info.path.join(">"),
            contexts.len()
        );*/
        if contexts.is_empty() {
            return Err(RuntimeError::RuntimeError {
                message: "No context! This is probably a bug, please contact sputnix".to_string(),
                info,
            });
        }
        use ast::StatementBody::*;

        let stored_context = if statement.arrow {
            Some(contexts.clone())
        } else {
            None
        };

        info.line = statement.line;
        use crate::fmt::SpwnFmt;
        match &statement.body {
            Expr(expr) => {
                let mut new_contexts: Vec<Context> = Vec::new();
                for context in contexts {
                    let is_assign = !expr.operators.is_empty()
                        && expr.operators[0] == ast::Operator::Assign
                        && !expr.values[0].is_defined(&context, globals);

                    if is_assign {
                        let mut new_expr = expr.clone();
                        let symbol = new_expr.values.remove(0);
                        new_expr.operators.remove(0); //assign operator
                        let constant = symbol.operator != Some(ast::UnaryOperator::Let);

                        //let mut new_context = context.clone();

                        match (new_expr.values.len() == 1, &new_expr.values[0].value.body) {
                            (true, ast::ValueBody::CmpStmt(f)) => {
                                //to account for recursion

                                //create the function context
                                let mut new_context = context.clone();
                                let storage = symbol.define(&mut new_context, globals, &info)?;

                                //pick a start group
                                let start_group = Group::next_free(&mut globals.closed_groups);
                                //store value
                                globals.stored_values[storage] =
                                    Value::Func(Function { start_group });

                                new_context.start_group = start_group;

                                let new_info = info.next(&symbol.fmt(0), globals, true);
                                let (_, inner_returns) = compile_scope(
                                    &f.statements,
                                    vec![new_context],
                                    globals,
                                    new_info,
                                )?;
                                returns.extend(inner_returns);

                                let mut after_context = context.clone();

                                let var_storage =
                                    symbol.define(&mut after_context, globals, &info)?;

                                globals.stored_values[var_storage] =
                                    Value::Func(Function { start_group });

                                new_contexts.push(after_context);
                            }
                            _ => {
                                let (evaled, inner_returns) =
                                    new_expr.eval(context, globals, info.clone(), constant)?;
                                returns.extend(inner_returns);
                                for (e, c2) in evaled {
                                    let mut new_context = c2.clone();
                                    let storage =
                                        symbol.define(&mut new_context, globals, &info)?;
                                    //clone the value so as to not share the reference
                                    globals.stored_values[storage] =
                                        globals.stored_values[e].clone();
                                    new_contexts.push(new_context);
                                }
                            }
                        }
                    } else {
                        //we dont care about the return value in this case
                        let (evaled, inner_returns) =
                            expr.eval(context, globals, info.clone(), false)?;
                        returns.extend(inner_returns);
                        new_contexts.extend(evaled.iter().map(|x| {
                            //globals.stored_values.map.remove(&x.0);
                            x.1.clone()
                        }));
                    }
                }
                contexts = new_contexts;
            }

            /*Definition(def) => {
                let mut all_values: Returns = Vec::new();

                for context in contexts {
                    if let ast::ValueBody::CmpStmt(f) = &def.value.values[0].value.body {
                        if def.value.values.len() == 1 {
                            //create the function context
                            let mut new_context = context.clone();
                            new_context.spawn_triggered = true;
                            //pick a start group
                            let start_group = Group {
                                id: next_free(
                                    &mut globals.closed_groups,
                                    ast::IDClass::Group,
                                    info.clone(),
                                )?,
                            };
                            let stored = store_const_value(
                                Value::Func(Function { start_group }),
                                1,
                                globals,
                                &context,
                            );
                            new_context.variables.insert(def.symbol.clone(), stored);
                            all_values.push((stored, context));
                            new_context.start_group = start_group;
                            let new_info = info.next(&def.symbol, globals, true);
                            let (_, inner_returns) =
                                compile_scope(&f.statements, vec![new_context], globals, new_info)?;
                            returns.extend(inner_returns);
                        } else {
                            let (evaled, inner_returns) =
                                def.value.eval(context, globals, info.clone())?;
                            returns.extend(inner_returns);
                            all_values.extend(evaled);
                        }
                    } else {
                        let (evaled, inner_returns) =
                            def.value.eval(context, globals, info.clone())?;
                        returns.extend(inner_returns);
                        all_values.extend(evaled);
                    }
                    //copied because im lazy
                }
                contexts = Vec::new();
                for (val, mut context) in all_values {
                    /*if !def.mutable {
                        (*globals.stored_values.map.get_mut(&val).unwrap()).2 = false;
                    }*/
                    context.variables.insert(String::from(&def.symbol), val);

                    contexts.push(context);
                }
            }*/
            Extract(val) => {
                let mut all_values: Returns = Vec::new();
                for context in contexts {
                    let (evaled, inner_returns) = val.eval(context, globals, info.clone(), true)?;
                    returns.extend(inner_returns);
                    all_values.extend(evaled);
                }

                contexts = Vec::new();
                for (val, mut context) in all_values {
                    match &globals.stored_values[val] {
                        Value::Dict(d) => {
                            context.variables.extend(d.clone());
                        }
                        Value::Builtins => {
                            for name in BUILTIN_LIST.iter() {
                                let p = store_value(
                                    Value::BuiltinFunction(String::from(*name)),
                                    1,
                                    globals,
                                    &context,
                                );

                                context.variables.insert(String::from(*name), p);
                            }
                        }
                        a => {
                            return Err(RuntimeError::RuntimeError {
                                message: format!(
                                    "This type ({}) can not be extracted!",
                                    a.to_str(globals)
                                ),
                                info,
                            })
                        }
                    }

                    contexts.push(context);
                }
            }

            TypeDef(name) => {
                //initialize type
                (*globals).type_id_count += 1;
                (*globals)
                    .type_ids
                    .insert(name.clone(), globals.type_id_count);
                //Value::TypeIndicator(globals.type_id_count)
            }

            If(if_stmt) => {
                let mut all_values: Returns = Vec::new();
                for context in contexts.clone() {
                    let new_info = info.next("if condition", globals, false);
                    let (evaled, inner_returns) =
                        if_stmt.condition.eval(context, globals, new_info, true)?;
                    returns.extend(inner_returns);
                    all_values.extend(evaled);
                }

                for (val, context) in all_values {
                    match &globals.stored_values[val] {
                        Value::Bool(b) => {
                            //internal if statement
                            if *b {
                                contexts = Vec::new();
                                let new_info = info.next("if body", globals, true);
                                let compiled = compile_scope(
                                    &if_stmt.if_body,
                                    vec![context],
                                    globals,
                                    new_info,
                                )?;
                                returns.extend(compiled.1);
                                contexts.extend(compiled.0);
                            } else {
                                match &if_stmt.else_body {
                                    Some(body) => {
                                        contexts = Vec::new();
                                        let new_info = info.next("else body", globals, true);
                                        let compiled =
                                            compile_scope(body, vec![context], globals, new_info)?;
                                        returns.extend(compiled.1);
                                        contexts.extend(compiled.0);
                                    }
                                    None => {}
                                };
                            }
                        }
                        a => {
                            return Err(RuntimeError::RuntimeError {
                                message: format!(
                                    "Expected boolean condition in if statement, found {}",
                                    a.to_str(globals)
                                ),
                                info,
                            })
                        }
                    }
                }
            }

            Impl(imp) => {
                let mut new_contexts: Vec<Context> = Vec::new();
                for context in contexts.clone() {
                    let new_info = info.next("implementation symbol", globals, false);
                    let (evaled, inner_returns) =
                        imp.symbol
                            .to_value(context.clone(), globals, new_info, true)?;
                    returns.extend(inner_returns);
                    for (typ, c) in evaled {
                        match globals.stored_values[typ].clone() {
                            Value::TypeIndicator(s) => {
                                let new_info = info.next("implementation", globals, true);
                                let (evaled, inner_returns) =
                                    eval_dict(imp.members.clone(), c, globals, new_info, true)?;

                                //Returns inside impl values dont really make sense do they
                                returns.extend(inner_returns);
                                for (val, c2) in evaled {
                                    globals.stored_values.increment_single_lifetime(val, 1000);
                                    let mut new_context = c2.clone();
                                    if let Value::Dict(d) = &globals.stored_values[val] {
                                        match new_context.implementations.get_mut(&s) {
                                            Some(implementation) => {
                                                for (key, val) in d.into_iter() {
                                                    (*implementation).insert(key.clone(), *val);
                                                }
                                            }
                                            None => {
                                                new_context
                                                    .implementations
                                                    .insert(s.clone(), d.clone());
                                            }
                                        }
                                    } else {
                                        unreachable!();
                                    }
                                    new_contexts.push(new_context);
                                }
                            }
                            a => {
                                return Err(RuntimeError::RuntimeError {
                                    message: format!(
                                        "Expected type-indicator, found {}",
                                        a.to_str(globals)
                                    ),
                                    info,
                                })
                            }
                        }
                    }
                }
                //println!("{:?}", new_contexts[0].implementations);
                contexts = new_contexts;
            }
            Call(call) => {
                /*for context in &mut contexts {
                    context.x += 1;
                }*/
                let mut all_values: Returns = Vec::new();
                for context in contexts {
                    let (evaled, inner_returns) =
                        call.function
                            .to_value(context, globals, info.clone(), true)?;
                    returns.extend(inner_returns);
                    all_values.extend(evaled);
                }
                contexts = Vec::new();
                let mut obj_list = Vec::<GDObj>::new();
                for (func, context) in all_values {
                    contexts.push(context.clone());
                    let mut params = HashMap::new();
                    params.insert(
                        51,
                        match &globals.stored_values[func] {
                            Value::Func(g) => ObjParam::Group(g.start_group),
                            Value::Group(g) => ObjParam::Group(*g),
                            a => {
                                return Err(RuntimeError::RuntimeError {
                                    message: format!(
                                        "Expected function or group, found: {}",
                                        a.to_str(globals)
                                    ),
                                    info,
                                })
                            }
                        },
                    );
                    params.insert(1, ObjParam::Number(1268.0));
                    obj_list.push(
                        GDObj {
                            params,

                            ..context_trigger(context.clone(), info.clone())
                        }
                        .context_parameters(context.clone()),
                    );
                }
                (*globals).func_ids[info.func_id].obj_list.extend(obj_list);
            }

            For(f) => {
                let mut all_arrays: Returns = Vec::new();
                for context in contexts {
                    let (evaled, inner_returns) =
                        f.array.eval(context, globals, info.clone(), true)?;
                    returns.extend(inner_returns);
                    all_arrays.extend(evaled);
                }
                contexts = Vec::new();
                for (val, context) in all_arrays {
                    match globals.stored_values[val].clone() {
                        Value::Array(arr) => {
                            //let iterator_val = store_value(Value::Null, globals);
                            //let scope_vars = context.variables.clone();

                            let mut new_contexts = vec![context.clone()];

                            for element in arr {
                                for mut c in new_contexts.clone() {
                                    c.variables = context.variables.clone();

                                    c.variables.insert(f.symbol.clone(), element);
                                    let new_info = info.next("for loop", globals, false);
                                    let (end_contexts, inner_returns) =
                                        compile_scope(&f.body, vec![c], globals, new_info)?;
                                    returns.extend(inner_returns);
                                    new_contexts = end_contexts;
                                }
                            }
                            contexts.extend(new_contexts);
                        }

                        a => {
                            return Err(RuntimeError::RuntimeError {
                                message: format!("{} is not iteratable!", a.to_str(globals)),
                                info,
                            })
                        }
                    }
                }
            }
            Return(return_val) => match return_val {
                Some(val) => {
                    let mut all_values: Returns = Vec::new();
                    for context in contexts.clone() {
                        let new_info = info.next("implementation symbol", globals, false);
                        let (evaled, inner_returns) = val.eval(context, globals, new_info, true)?;
                        returns.extend(inner_returns);
                        all_values.extend(evaled);
                    }

                    returns.extend(all_values);
                }

                None => {
                    let mut all_values: Returns = Vec::new();
                    for context in contexts.clone() {
                        all_values.push((store_value(Value::Null, 1, globals, &context), context));
                    }
                    returns.extend(all_values);
                }
            },

            Error(e) => {
                for context in contexts.clone() {
                    let new_info = info.next("return value", globals, false);
                    let (evaled, _) = e.message.eval(context, globals, new_info, true)?;
                    for (msg, _) in evaled {
                        eprintln!(
                            "ERROR: {:?}",
                            match &globals.stored_values[msg] {
                                Value::Str(s) => s,
                                _ => "no message",
                            }
                        );
                    }
                }
                return Err(RuntimeError::RuntimeError {
                    message: "Error statement, see message(s) above.".to_string(),
                    info,
                });
            }
        }
        if let Some(c) = stored_context {
            //resetting the context if async
            contexts = c;
        }

        /*println!(
            "{} -> Compiled '{}' in {} milliseconds!",
            path,
            statement_type,
            start_time.elapsed().as_millis(),
        );*/
    }

    //return values need longer lifetimes
    for (val, _) in &returns {
        globals.stored_values.increment_single_lifetime(*val, 1);
    }

    globals.stored_values.decrement_lifetimes();
    //collect garbage
    globals.stored_values.clean_up();

    //(*globals).highest_x = context.x;
    Ok((contexts, returns))
}

fn merge_impl(target: &mut Implementations, source: &Implementations) {
    for (key, imp) in source.iter() {
        match target.get_mut(key) {
            Some(target_imp) => {
                (*target_imp).extend(imp.iter().map(|x| (x.0.clone(), x.1.clone())))
            }
            None => {
                (*target).insert(*key, imp.clone());
            }
        }
    }
}

pub fn import_module(
    path: &PathBuf,
    context: &Context,
    globals: &mut Globals,
    info: CompilerInfo,
) -> Result<Returns, RuntimeError> {
    let mut module_path = globals
        .path
        .clone()
        .parent()
        .expect("Your file must be in a folder to import modules!")
        .join(&path);

    if module_path.is_dir() {
        module_path = module_path.join("lib.spwn");
    }

    let unparsed = match fs::read_to_string(&module_path) {
        Ok(content) => content,
        Err(e) => {
            return Err(RuntimeError::RuntimeError {
                message: format!(
                    "Something went wrong when opening library file ({:?}): {}",
                    module_path, e
                ),
                info,
            })
        }
    };
    let (parsed, notes) = match crate::parse_spwn(unparsed) {
        Ok(p) => p,
        Err(err) => return Err(RuntimeError::PackageSyntaxError { err, info }),
    };
    // (*globals).closed_groups.extend(notes.closed_groups);
    // (*globals).closed_colors.extend(notes.closed_colors);
    // (*globals).closed_blocks.extend(notes.closed_blocks);
    // (*globals).closed_items.extend(notes.closed_items);

    let stored_path = globals.path.clone();
    (*globals).path = module_path;

    let new_info = info.next("module", globals, false);
    let (contexts, returns) = compile_scope(&parsed, vec![Context::new()], globals, new_info)?;
    (*globals).path = stored_path;

    Ok(if returns.is_empty() {
        contexts
            .iter()
            .map(|x| {
                let mut new_context = context.clone();
                new_context.spawn_triggered = x.spawn_triggered;
                new_context.start_group = x.start_group;
                merge_impl(&mut new_context.implementations, &x.implementations);
                (NULL_STORAGE, new_context)
            })
            .collect()
    } else {
        returns
            .iter()
            .map(|(val, x)| {
                let mut new_context = context.clone();
                new_context.spawn_triggered = x.spawn_triggered;
                new_context.start_group = x.start_group;
                merge_impl(&mut new_context.implementations, &x.implementations);
                (*val, new_context)
            })
            .collect()
    })
}

// const ID_MAX: u16 = 999;

// pub fn next_free(
//     ids: &mut Vec<u16>,
//     id_class: ast::IDClass,
//     info: CompilerInfo,
// ) -> Result<ID, RuntimeError> {
//     for i in 1..ID_MAX {
//         if !ids.contains(&i) {
//             (*ids).push(i);
//             return Ok(i);
//         }
//     }

//     Err(RuntimeError::IDError { id_class, info })
//     //panic!("All ids of this type are used up!");
// }
