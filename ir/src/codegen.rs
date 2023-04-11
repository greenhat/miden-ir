use crate::miden::{ProgramAst, ProcedureAst, Node, Instruction}; //TODO: Fix this
use anyhow::Result; //TODO: This might be unnecessary

impl Pass {
    type Input = Program;
    type Output = miden::ProgramAst;

    pub fn run(input : Program) -> anyhow::Result<ProgramAst, CompilerError> {
	codegen_program(input);//TODO: use Result
    }

    fn codegen_program (program: Program) -> Result<ProgramAst, CompilerError> {
	//let functions = foreach f : program.functions { codegen_function(f) }
	let main_res = codegen_function(program.main_function);
	ProgramAst {
	    local_procs = Vec::new(), //Vec::new(functions)
	    body = main_res.body,  //TODO: Clone
	}
    }

    fn codegen_function (function: Function) -> Result<ProcedureAst, CompilerError> {
	
    }
}

