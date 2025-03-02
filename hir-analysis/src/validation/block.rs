use miden_diagnostics::{DiagnosticsHandler, Severity, SourceSpan, Spanned};
use miden_hir::*;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;

use super::{Rule, ValidationError};
use crate::DominatorTree;

/// This validation rule ensures that all values definitions dominate their uses.
///
/// For example, it is not valid to use a value in a block when its definition only
/// occurs along a subset of control flow paths which may be taken to that block.
///
/// This also catches uses of values which are orphaned (i.e. they are defined by
/// a block parameter or instruction which is not attached to the function).
pub struct DefsDominateUses<'a> {
    dfg: &'a DataFlowGraph,
    domtree: &'a DominatorTree,
}
impl<'a> DefsDominateUses<'a> {
    pub fn new(dfg: &'a DataFlowGraph, domtree: &'a DominatorTree) -> Self {
        Self { dfg, domtree }
    }
}
impl<'a> Rule<BlockData> for DefsDominateUses<'a> {
    fn validate(
        &mut self,
        block_data: &BlockData,
        diagnostics: &DiagnosticsHandler,
    ) -> Result<(), ValidationError> {
        let current_block = block_data.id;
        let mut uses = FxHashSet::<Value>::default();
        let mut defs = FxHashSet::<Value>::default();
        for node in block_data.insts.iter() {
            let span = node.span();

            uses.clear();
            defs.clear();

            // Verify the integrity of the instruction results
            for result in self.dfg.inst_results(node.key) {
                // It should never be possible for a value id to be present in the result set twice
                assert!(defs.insert(*result));
            }

            // Gather all value uses to check
            uses.extend(node.arguments(&self.dfg.value_lists).iter().copied());
            match node.analyze_branch(&self.dfg.value_lists) {
                BranchInfo::NotABranch => (),
                BranchInfo::SingleDest(_, args) => {
                    uses.extend(args.iter().copied());
                }
                BranchInfo::MultiDest(ref jts) => {
                    for jt in jts.iter() {
                        uses.extend(jt.args.iter().copied());
                    }
                }
            }

            // Make sure there are no uses of the instructions own results
            if !defs.is_disjoint(&uses) {
                invalid_instruction!(
                    diagnostics,
                    node.key,
                    span,
                    "an instruction may not use its own results as arguments",
                    "This situation can only arise if one has manually modified the arguments of an instruction, \
                     incorrectly inserting a value obtained from the set of instruction results."
                );
            }

            // Next, ensure that all used values are dominated by their definition
            for value in uses.iter().copied() {
                match self.dfg.value_data(value) {
                    // If the value comes from the current block's parameter list, this use is trivially dominated
                    ValueData::Param { block, .. } if block == &current_block => continue,
                    // If the value comes from another block, then as long as all paths to the current
                    // block flow through that block, then this use is dominated by its definition
                    ValueData::Param { block, .. } => {
                        if self.domtree.dominates(*block, current_block, &self.dfg) {
                            continue;
                        }
                    }
                    // If the value is an instruction result, then as long as all paths to the current
                    // instruction flow through the defining instruction, then this use is dominated
                    // by its definition
                    ValueData::Inst { inst, .. } => {
                        if self.domtree.dominates(*inst, node.key, &self.dfg) {
                            continue;
                        }
                    }
                }

                // If we reach here, the use of `value` is not dominated by its definition,
                // so this use is invalid
                invalid_instruction!(
                    diagnostics,
                    node.key,
                    span,
                    "an argument of this instruction, {value}, is not defined on all paths leading to this point",
                    "All uses of a value must be dominated by its definition, i.e. all control flow paths \
                     from the function entry to the point of each use must flow through the point where \
                     that value is defined."
                );
            }
        }

        Ok(())
    }
}

/// This validation rule ensures that most block-local invariants are upheld:
///
/// * A block may not be empty
/// * A block must end with a terminator instruction
/// * A block may not contain a terminator instruction in any position but the end
/// * A block which terminates with a branch instruction must reference a block
///   that is present in the function body (i.e. it is not valid to reference
///   detached blocks)
/// * A multi-way branch instruction must have at least one successor
/// * A multi-way branch instruction must not specify the same block as a successor multiple times.
///
/// This rule does not perform type checking, or verify use/def dominance.
pub struct BlockValidator<'a> {
    dfg: &'a DataFlowGraph,
    span: SourceSpan,
}
impl<'a> BlockValidator<'a> {
    pub fn new(dfg: &'a DataFlowGraph, span: SourceSpan) -> Self {
        Self { dfg, span }
    }
}
impl<'a> Rule<BlockData> for BlockValidator<'a> {
    fn validate(
        &mut self,
        block_data: &BlockData,
        diagnostics: &DiagnosticsHandler,
    ) -> Result<(), ValidationError> {
        // Ignore blocks which are not attached to the function body
        if !block_data.link.is_linked() {
            return Ok(());
        }

        // Ensure there is a terminator, and that it is valid
        let id = block_data.id;
        let terminator = block_data.insts.back().get();
        if terminator.is_none() {
            // This block is empty
            invalid_block!(
                diagnostics,
                id,
                self.span,
                "block cannot be empty",
                "Empty blocks are only valid when detached from the function body"
            );
        }

        let terminator = terminator.unwrap();
        let op = terminator.opcode();
        if !op.is_terminator() {
            invalid_block!(
                diagnostics,
                id,
                self.span,
                "invalid block terminator",
                format!("The last instruction in a block must be a terminator, but {id} ends with {op} which is not a valid terminator")
            );
        }
        match terminator.analyze_branch(&self.dfg.value_lists) {
            BranchInfo::SingleDest(destination, _) => {
                let dest = self.dfg.block(destination);
                if !dest.link.is_linked() {
                    invalid_instruction!(
                        diagnostics,
                        terminator.key,
                        terminator.span(),
                        "invalid successor",
                        format!("A block reference is only valid if the referenced block is present in the function layout. \
                                 {id} references {destination}, but the latter is not in the layout")
                    );
                }
            }
            BranchInfo::MultiDest(ref jts) => {
                if jts.is_empty() {
                    invalid_instruction!(
                        diagnostics,
                        terminator.key,
                        terminator.span(),
                        "incomplete {op} instruction",
                        "This instruction normally has 2 or more successors, but none were given."
                    );
                }

                let mut seen = SmallVec::<[Block; 4]>::default();
                for jt in jts.iter() {
                    let dest = self.dfg.block(jt.destination);
                    let destination = jt.destination;
                    if !dest.link.is_linked() {
                        invalid_instruction!(
                            diagnostics,
                            terminator.key,
                            terminator.span(),
                            "invalid successor",
                            format!("A block reference is only valid if the referenced block is present in the function layout. \
                                    {id} references {destination}, but the latter is not in the layout")
                        );
                    }

                    if seen.contains(&jt.destination) {
                        invalid_instruction!(
                            diagnostics,
                            terminator.key,
                            terminator.span(),
                            "invalid {op} instruction",
                            format!("A given block may only be a successor along a single control flow path, \
                                     but {id} uses {destination} as a successor for more than one path")
                        );
                    }

                    seen.push(jt.destination);
                }
            }
            BranchInfo::NotABranch => (),
        }

        // Verify that there are no terminator instructions in any other position than last
        for node in block_data.insts.iter() {
            let op = node.opcode();
            if op.is_terminator() && node.key != terminator.key {
                invalid_block!(
                    diagnostics,
                    id,
                    self.span,
                    "terminator found in middle of block",
                    format!("A block may only have a terminator instruction as the last instruction in the block, \
                             but {id} uses {op} before the end of the block")
                );
            }
        }

        Ok(())
    }
}
