//! Deterministic JSONL chunk witnesses for sync pipelines.
//!
//! This module is intentionally pure: it reads JSONL bytes and produces a
//! stable witness without touching files, paths, storage, or git state. The
//! serial import/export path can keep its existing behavior while future
//! parallel sync work uses these witnesses to prove unchanged chunks.

use crate::util::hex_encode;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{self, BufRead, Read, Seek, SeekFrom, Write},
    sync::mpsc,
    thread,
};

/// Schema marker for JSONL Merkle witnesses.
pub const JSONL_WITNESS_SCHEMA_VERSION: &str = "obr.jsonl-witness.v1";

const ROOT_DOMAIN: &[u8] = b"obr:jsonl-witness:root:v1\0";
const CHUNK_DOMAIN: &[u8] = b"obr:jsonl-witness:chunk:v1\0";
const FIELD_SEPARATOR: &[u8] = b"\0";

/// Merkle-style witness for an exact JSONL byte stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlMerkleWitness {
    pub schema_version: String,
    pub chunk_size_lines: usize,
    pub line_count: usize,
    pub byte_count: u64,
    pub root_hash: String,
    pub chunks: Vec<JsonlChunkWitness>,
}

/// Witness metadata for one contiguous JSONL line chunk.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlChunkWitness {
    pub index: usize,
    pub start_line: usize,
    pub line_count: usize,
    pub byte_count: u64,
    pub hash: String,
    pub first_line_hash: Option<String>,
    pub last_line_hash: Option<String>,
}

/// Drift summary between two JSONL Merkle witnesses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[must_use = "comparison summaries identify which chunks can be reused safely"]
pub struct JsonlWitnessComparison {
    pub schema_versions_match: bool,
    pub chunk_size_lines_match: bool,
    pub root_hashes_match: bool,
    pub drift_detected: bool,
    pub base_line_count: usize,
    pub candidate_line_count: usize,
    pub base_byte_count: u64,
    pub candidate_byte_count: u64,
    pub base_chunk_count: usize,
    pub candidate_chunk_count: usize,
    pub comparable_chunk_count: usize,
    pub unchanged_chunks: usize,
    pub changed_chunks: usize,
    pub added_chunks: usize,
    pub removed_chunks: usize,
    pub unchanged_byte_count: u64,
    pub changed_base_byte_count: u64,
    pub changed_candidate_byte_count: u64,
    pub added_byte_count: u64,
    pub removed_byte_count: u64,
    pub safe_reuse_prefix_chunks: usize,
    pub first_changed_chunk_index: Option<usize>,
}

/// Candidate-ordered reuse plan for a future parallel JSONL sync pipeline.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[must_use = "reuse plans identify which chunks can be copied or rebuilt"]
pub struct JsonlWitnessReusePlan {
    pub comparison: JsonlWitnessComparison,
    pub schedule: JsonlWitnessReuseSchedule,
    pub actions: Vec<JsonlChunkReuseStep>,
}

/// Proof summary from replaying a JSONL witness reuse plan into candidate bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[must_use = "materialization summaries prove how many chunks were reused or rebuilt"]
pub struct JsonlWitnessReuseMaterialization {
    pub reused_chunks: usize,
    pub rebuilt_chunks: usize,
    pub read_added_chunks: usize,
    pub dropped_chunks: usize,
    pub output_byte_count: u64,
    pub reused_byte_count: u64,
    pub rebuilt_byte_count: u64,
    pub read_added_byte_count: u64,
    pub dropped_byte_count: u64,
}

impl JsonlWitnessReuseMaterialization {
    fn record_reused(&mut self, byte_count: u64) -> io::Result<()> {
        self.reused_chunks += 1;
        self.reused_byte_count =
            checked_add_u64(self.reused_byte_count, byte_count, "reused byte count")?;
        self.record_output_bytes(byte_count)
    }

    fn record_rebuilt(&mut self, byte_count: u64) -> io::Result<()> {
        self.rebuilt_chunks += 1;
        self.rebuilt_byte_count =
            checked_add_u64(self.rebuilt_byte_count, byte_count, "rebuilt byte count")?;
        self.record_output_bytes(byte_count)
    }

    fn record_read_added(&mut self, byte_count: u64) -> io::Result<()> {
        self.read_added_chunks += 1;
        self.read_added_byte_count = checked_add_u64(
            self.read_added_byte_count,
            byte_count,
            "read-added byte count",
        )?;
        self.record_output_bytes(byte_count)
    }

    fn record_dropped(&mut self, byte_count: u64) -> io::Result<()> {
        self.dropped_chunks += 1;
        self.dropped_byte_count =
            checked_add_u64(self.dropped_byte_count, byte_count, "dropped byte count")?;
        Ok(())
    }

    fn record_output_bytes(&mut self, byte_count: u64) -> io::Result<()> {
        self.output_byte_count = checked_add_u64(
            self.output_byte_count,
            byte_count,
            "materialized output byte count",
        )?;
        Ok(())
    }
}

/// Bounded work plan for executing a witness reuse plan with parallel workers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlWitnessParallelWorkPlan {
    pub max_parallelism: usize,
    pub total_batches: usize,
    pub candidate_output_batches: usize,
    pub metadata_only_drop_batches: usize,
    pub deterministic_batch_order: bool,
    pub batches: Vec<JsonlWitnessWorkBatch>,
}

/// One bounded batch of independent JSONL witness work.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlWitnessWorkBatch {
    pub index: usize,
    pub kind: JsonlWitnessWorkBatchKind,
    pub action_count: usize,
    pub candidate_start_index: Option<usize>,
    pub candidate_end_index: Option<usize>,
    pub line_count: usize,
    pub byte_count: u64,
    pub actions: Vec<JsonlChunkReuseStep>,
}

/// Batch class for a parallel witness work plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonlWitnessWorkBatchKind {
    CandidateOutput,
    MetadataOnlyDrop,
}

/// Scheduling summary derived from a candidate-ordered JSONL reuse plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlWitnessReuseSchedule {
    pub total_actions: usize,
    pub candidate_output_actions: usize,
    pub metadata_only_drop_actions: usize,
    pub reusable_actions: usize,
    pub rebuild_actions: usize,
    pub read_added_actions: usize,
    pub candidate_output_line_count: usize,
    pub candidate_output_byte_count: u64,
    pub reusable_byte_count: u64,
    pub rebuild_byte_count: u64,
    pub read_added_byte_count: u64,
    pub dropped_byte_count: u64,
    pub max_parallel_candidate_actions: usize,
    pub deterministic_candidate_order: bool,
}

/// One chunk action in a JSONL witness reuse plan.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JsonlChunkReuseStep {
    pub action: JsonlChunkReuseAction,
    pub base_index: Option<usize>,
    pub candidate_index: Option<usize>,
    pub start_line: usize,
    pub line_count: usize,
    pub byte_count: u64,
}

/// Conservative work class for one witness chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JsonlChunkReuseAction {
    ReuseUnchanged,
    RebuildCandidate,
    ReadAdded,
    DropRemoved,
}

/// Build a deterministic witness over exact JSONL bytes read from `reader`.
///
/// Lines are counted by `BufRead::read_until(b'\n')`, so newline bytes are part
/// of the line hash when present and the final non-newline-terminated line is
/// still counted.
pub fn build_jsonl_merkle_witness<R: BufRead>(
    mut reader: R,
    chunk_size_lines: usize,
) -> io::Result<JsonlMerkleWitness> {
    if chunk_size_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chunk_size_lines must be greater than zero",
        ));
    }

    let mut chunks = Vec::new();
    let mut line_count = 0_usize;
    let mut byte_count = 0_u64;
    let mut chunk_builder = ChunkBuilder::new(0, 0)?;
    let mut line = Vec::new();

    loop {
        line.clear();
        let bytes_read = reader.read_until(b'\n', &mut line)?;
        if bytes_read == 0 {
            break;
        }

        if chunk_builder.line_count == chunk_size_lines {
            chunks.push(chunk_builder.finish());
            chunk_builder = ChunkBuilder::new(chunks.len(), line_count)?;
        }

        line_count = line_count
            .checked_add(1)
            .ok_or_else(|| io_invalid_data("JSONL line count overflowed usize"))?;
        byte_count = checked_add_bytes(byte_count, bytes_read, "JSONL byte count")?;
        chunk_builder.push_line(&line, bytes_read)?;
    }

    if chunk_builder.line_count > 0 {
        chunks.push(chunk_builder.finish());
    }

    let root_hash = compute_root_hash(chunk_size_lines, line_count, byte_count, &chunks)?;

    Ok(JsonlMerkleWitness {
        schema_version: JSONL_WITNESS_SCHEMA_VERSION.to_string(),
        chunk_size_lines,
        line_count,
        byte_count,
        root_hash,
        chunks,
    })
}

/// Build a deterministic JSONL witness with bounded parallel chunk hashing.
///
/// The input stream is still read in order, but complete chunks are sent to a
/// fixed worker set and then reassembled by chunk index before computing the
/// root hash. `max_parallelism == 1` uses the serial implementation exactly.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when `chunk_size_lines` or
/// `max_parallelism` is zero. Returns I/O errors from the reader or worker
/// coordination.
pub fn build_jsonl_merkle_witness_parallel<R: BufRead>(
    reader: R,
    chunk_size_lines: usize,
    max_parallelism: usize,
) -> io::Result<JsonlMerkleWitness> {
    if max_parallelism == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "max_parallelism must be greater than zero",
        ));
    }
    if max_parallelism == 1 {
        return build_jsonl_merkle_witness(reader, chunk_size_lines);
    }
    if chunk_size_lines == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "chunk_size_lines must be greater than zero",
        ));
    }

    let (chunks, line_count, byte_count) =
        build_parallel_chunks(reader, chunk_size_lines, max_parallelism)?;
    let root_hash = compute_root_hash(chunk_size_lines, line_count, byte_count, &chunks)?;

    Ok(JsonlMerkleWitness {
        schema_version: JSONL_WITNESS_SCHEMA_VERSION.to_string(),
        chunk_size_lines,
        line_count,
        byte_count,
        root_hash,
        chunks,
    })
}

/// Build a conservative, candidate-ordered chunk reuse plan.
///
/// Emitting actions appear in candidate JSONL order, so workers may process
/// reusable and rebuild chunks independently while the coordinator emits by
/// `candidate_index`. `DropRemoved` actions are appended after emitting actions
/// because they have no candidate output position.
pub fn plan_jsonl_witness_reuse(
    base: &JsonlMerkleWitness,
    candidate: &JsonlMerkleWitness,
) -> JsonlWitnessReusePlan {
    let comparison = compare_jsonl_merkle_witnesses(base, candidate);
    let witnesses_are_comparable =
        comparison.schema_versions_match && comparison.chunk_size_lines_match;
    let comparable_chunk_count = if witnesses_are_comparable {
        base.chunks.len().min(candidate.chunks.len())
    } else {
        0
    };
    let mut actions = Vec::with_capacity(base.chunks.len().max(candidate.chunks.len()));

    if witnesses_are_comparable {
        for (index, (base_chunk, candidate_chunk)) in
            base.chunks.iter().zip(&candidate.chunks).enumerate()
        {
            let action = if chunks_match_for_reuse(base_chunk, candidate_chunk) {
                JsonlChunkReuseAction::ReuseUnchanged
            } else {
                JsonlChunkReuseAction::RebuildCandidate
            };
            actions.push(candidate_chunk_step(
                action,
                Some(index),
                index,
                candidate_chunk,
            ));
        }
    }

    for (offset, chunk) in candidate
        .chunks
        .get(comparable_chunk_count..)
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        actions.push(candidate_chunk_step(
            if witnesses_are_comparable {
                JsonlChunkReuseAction::ReadAdded
            } else {
                JsonlChunkReuseAction::RebuildCandidate
            },
            None,
            comparable_chunk_count + offset,
            chunk,
        ));
    }

    for (offset, chunk) in base
        .chunks
        .get(comparable_chunk_count..)
        .unwrap_or(&[])
        .iter()
        .enumerate()
    {
        actions.push(base_chunk_step(
            JsonlChunkReuseAction::DropRemoved,
            comparable_chunk_count + offset,
            chunk,
        ));
    }

    let schedule = build_reuse_schedule(&actions);
    JsonlWitnessReusePlan {
        comparison,
        schedule,
        actions,
    }
}

/// Split a candidate-ordered reuse plan into bounded parallel work batches.
///
/// Candidate-output batches always appear in candidate index order. Metadata-only
/// drops are emitted after candidate-output batches because they have no output
/// position in the candidate JSONL stream.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when `max_parallelism` is zero.
pub fn plan_jsonl_witness_parallel_work(
    plan: &JsonlWitnessReusePlan,
    max_parallelism: usize,
) -> io::Result<JsonlWitnessParallelWorkPlan> {
    if max_parallelism == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "max_parallelism must be greater than zero",
        ));
    }

    let mut batches = Vec::new();
    let mut current_kind = None;
    let mut current_actions = Vec::new();

    for step in &plan.actions {
        let step_kind = work_batch_kind(step);
        if let Some(kind) = current_kind {
            if kind != step_kind || current_actions.len() == max_parallelism {
                push_work_batch(&mut batches, kind, std::mem::take(&mut current_actions));
                current_kind = Some(step_kind);
            }
        } else {
            current_kind = Some(step_kind);
        }

        current_actions.push(step.clone());
    }

    if let Some(kind) = current_kind {
        push_work_batch(&mut batches, kind, current_actions);
    }

    let candidate_output_batches = batches
        .iter()
        .filter(|batch| matches!(batch.kind, JsonlWitnessWorkBatchKind::CandidateOutput))
        .count();
    let metadata_only_drop_batches = batches
        .iter()
        .filter(|batch| matches!(batch.kind, JsonlWitnessWorkBatchKind::MetadataOnlyDrop))
        .count();

    Ok(JsonlWitnessParallelWorkPlan {
        max_parallelism,
        total_batches: batches.len(),
        candidate_output_batches,
        metadata_only_drop_batches,
        deterministic_batch_order: plan.schedule.deterministic_candidate_order
            && work_batches_are_deterministically_ordered(&batches),
        batches,
    })
}

/// Replay a witness reuse plan into exact candidate JSONL bytes.
///
/// Unchanged chunks are copied from `base_reader`; changed and added candidate
/// chunks are copied from `candidate_reader`; removed base chunks are counted as
/// metadata-only drops. The inputs are seek-based so the planner can prove a
/// future parallel executor without buffering the full JSONL stream.
///
/// Returns [`io::ErrorKind::InvalidData`] when the plan does not match the two
/// witnesses, when candidate-output actions are not in deterministic candidate
/// order, or when a reuse action tries to copy a chunk whose witness metadata no
/// longer matches.
pub fn materialize_jsonl_witness_reuse_plan<RB, RC, W>(
    base_reader: &mut RB,
    candidate_reader: &mut RC,
    writer: &mut W,
    base_witness: &JsonlMerkleWitness,
    candidate_witness: &JsonlMerkleWitness,
    plan: &JsonlWitnessReusePlan,
) -> io::Result<JsonlWitnessReuseMaterialization>
where
    RB: Read + Seek,
    RC: Read + Seek,
    W: Write,
{
    validate_reuse_materialization_inputs(base_witness, candidate_witness, plan)?;

    let base_offsets = chunk_start_offsets(base_witness, "base")?;
    let candidate_offsets = chunk_start_offsets(candidate_witness, "candidate")?;
    let mut materialization = JsonlWitnessReuseMaterialization::default();

    for step in &plan.actions {
        match step.action {
            JsonlChunkReuseAction::ReuseUnchanged => {
                let base_index = required_chunk_index(step.base_index, "reuse base_index")?;
                let candidate_index =
                    required_chunk_index(step.candidate_index, "reuse candidate_index")?;
                let base_chunk = witness_chunk(&base_witness.chunks, base_index, "base")?;
                let candidate_chunk =
                    witness_chunk(&candidate_witness.chunks, candidate_index, "candidate")?;
                validate_step_matches_chunk(step, candidate_chunk, "candidate")?;
                if !chunks_match_for_reuse(base_chunk, candidate_chunk) {
                    return Err(io_invalid_data(
                        "reuse step no longer matches base and candidate chunk witnesses",
                    ));
                }
                let copied = copy_witness_chunk(
                    base_reader,
                    &base_offsets,
                    &base_witness.chunks,
                    base_index,
                    writer,
                    "base",
                )?;
                materialization.record_reused(copied)?;
            }
            JsonlChunkReuseAction::RebuildCandidate => {
                let candidate_index =
                    required_chunk_index(step.candidate_index, "rebuild candidate_index")?;
                let candidate_chunk =
                    witness_chunk(&candidate_witness.chunks, candidate_index, "candidate")?;
                validate_step_matches_chunk(step, candidate_chunk, "candidate")?;
                let copied = copy_witness_chunk(
                    candidate_reader,
                    &candidate_offsets,
                    &candidate_witness.chunks,
                    candidate_index,
                    writer,
                    "candidate",
                )?;
                materialization.record_rebuilt(copied)?;
            }
            JsonlChunkReuseAction::ReadAdded => {
                let candidate_index =
                    required_chunk_index(step.candidate_index, "read-added candidate_index")?;
                if step.base_index.is_some() {
                    return Err(io_invalid_data(
                        "read-added step unexpectedly referenced a base chunk",
                    ));
                }
                let candidate_chunk =
                    witness_chunk(&candidate_witness.chunks, candidate_index, "candidate")?;
                validate_step_matches_chunk(step, candidate_chunk, "candidate")?;
                let copied = copy_witness_chunk(
                    candidate_reader,
                    &candidate_offsets,
                    &candidate_witness.chunks,
                    candidate_index,
                    writer,
                    "candidate",
                )?;
                materialization.record_read_added(copied)?;
            }
            JsonlChunkReuseAction::DropRemoved => {
                let base_index = required_chunk_index(step.base_index, "drop base_index")?;
                if step.candidate_index.is_some() {
                    return Err(io_invalid_data(
                        "drop step unexpectedly referenced a candidate chunk",
                    ));
                }
                let base_chunk = witness_chunk(&base_witness.chunks, base_index, "base")?;
                validate_step_matches_chunk(step, base_chunk, "base")?;
                materialization.record_dropped(step.byte_count)?;
            }
        }
    }

    if materialization.output_byte_count != candidate_witness.byte_count {
        return Err(io_invalid_data(format!(
            "materialized output byte count {} did not match candidate witness byte count {}",
            materialization.output_byte_count, candidate_witness.byte_count
        )));
    }

    Ok(materialization)
}

/// Compare two JSONL witnesses and conservatively identify reusable chunks.
///
/// Chunks are considered reusable only when schema, chunk sizing, chunk index,
/// line range, byte count, and chunk hash all match. This intentionally treats
/// shifted chunks as changed so an incremental sync planner cannot hide drift
/// behind a coincidental hash match.
pub fn compare_jsonl_merkle_witnesses(
    base: &JsonlMerkleWitness,
    candidate: &JsonlMerkleWitness,
) -> JsonlWitnessComparison {
    let schema_versions_match = base.schema_version == candidate.schema_version;
    let chunk_size_lines_match = base.chunk_size_lines == candidate.chunk_size_lines;
    let root_hashes_match = base.root_hash == candidate.root_hash;
    let witnesses_are_comparable = schema_versions_match && chunk_size_lines_match;

    let mut unchanged_chunks = 0;
    let mut changed_chunks = 0;
    let mut unchanged_byte_count = 0;
    let mut changed_base_byte_count = 0;
    let mut changed_candidate_byte_count = 0;
    let mut safe_reuse_prefix_chunks = 0;
    let mut first_changed_chunk_index = None;

    let comparable_chunk_count = if witnesses_are_comparable {
        let comparable_chunk_count = base.chunks.len().min(candidate.chunks.len());
        let mut prefix_is_reusable = true;

        for (index, (base_chunk, candidate_chunk)) in
            base.chunks.iter().zip(&candidate.chunks).enumerate()
        {
            if chunks_match_for_reuse(base_chunk, candidate_chunk) {
                unchanged_chunks += 1;
                unchanged_byte_count += candidate_chunk.byte_count;
                if prefix_is_reusable {
                    safe_reuse_prefix_chunks += 1;
                }
            } else {
                changed_chunks += 1;
                changed_base_byte_count += base_chunk.byte_count;
                changed_candidate_byte_count += candidate_chunk.byte_count;
                prefix_is_reusable = false;
                first_changed_chunk_index.get_or_insert(index);
            }
        }

        comparable_chunk_count
    } else {
        0
    };

    let (added_chunks, removed_chunks, added_byte_count, removed_byte_count) =
        if witnesses_are_comparable {
            (
                candidate.chunks.len().saturating_sub(base.chunks.len()),
                base.chunks.len().saturating_sub(candidate.chunks.len()),
                sum_chunk_bytes(
                    candidate
                        .chunks
                        .get(comparable_chunk_count..)
                        .unwrap_or(&[]),
                ),
                sum_chunk_bytes(base.chunks.get(comparable_chunk_count..).unwrap_or(&[])),
            )
        } else {
            (
                candidate.chunks.len(),
                base.chunks.len(),
                sum_chunk_bytes(&candidate.chunks),
                sum_chunk_bytes(&base.chunks),
            )
        };

    if first_changed_chunk_index.is_none()
        && (!witnesses_are_comparable || added_chunks > 0 || removed_chunks > 0)
    {
        first_changed_chunk_index = Some(comparable_chunk_count);
    }

    JsonlWitnessComparison {
        schema_versions_match,
        chunk_size_lines_match,
        root_hashes_match,
        drift_detected: !(witnesses_are_comparable && root_hashes_match),
        base_line_count: base.line_count,
        candidate_line_count: candidate.line_count,
        base_byte_count: base.byte_count,
        candidate_byte_count: candidate.byte_count,
        base_chunk_count: base.chunks.len(),
        candidate_chunk_count: candidate.chunks.len(),
        comparable_chunk_count,
        unchanged_chunks,
        changed_chunks,
        added_chunks,
        removed_chunks,
        unchanged_byte_count,
        changed_base_byte_count,
        changed_candidate_byte_count,
        added_byte_count,
        removed_byte_count,
        safe_reuse_prefix_chunks,
        first_changed_chunk_index,
    }
}

fn chunks_match_for_reuse(base: &JsonlChunkWitness, candidate: &JsonlChunkWitness) -> bool {
    base.index == candidate.index
        && base.start_line == candidate.start_line
        && base.line_count == candidate.line_count
        && base.byte_count == candidate.byte_count
        && base.hash == candidate.hash
}

fn validate_reuse_materialization_inputs(
    base_witness: &JsonlMerkleWitness,
    candidate_witness: &JsonlMerkleWitness,
    plan: &JsonlWitnessReusePlan,
) -> io::Result<()> {
    validate_chunk_sequence(base_witness)?;
    validate_chunk_sequence(candidate_witness)?;

    let expected_comparison = compare_jsonl_merkle_witnesses(base_witness, candidate_witness);
    if plan.comparison != expected_comparison {
        return Err(io_invalid_data(
            "reuse plan comparison did not match supplied witnesses",
        ));
    }

    let expected_schedule = build_reuse_schedule(&plan.actions);
    if plan.schedule != expected_schedule {
        return Err(io_invalid_data(
            "reuse plan schedule did not match supplied actions",
        ));
    }

    if !candidate_actions_are_deterministically_ordered(&plan.actions) {
        return Err(io_invalid_data(
            "reuse plan candidate-output actions were not in deterministic candidate order",
        ));
    }

    if plan.schedule.candidate_output_actions != candidate_witness.chunks.len() {
        return Err(io_invalid_data(format!(
            "reuse plan had {} candidate-output actions for {} candidate chunks",
            plan.schedule.candidate_output_actions,
            candidate_witness.chunks.len()
        )));
    }

    Ok(())
}

fn validate_chunk_sequence(witness: &JsonlMerkleWitness) -> io::Result<()> {
    for (expected_index, chunk) in witness.chunks.iter().enumerate() {
        if chunk.index != expected_index {
            return Err(io_invalid_data(
                "witness chunks were not stored in ascending chunk-index order",
            ));
        }
    }
    Ok(())
}

fn chunk_start_offsets(witness: &JsonlMerkleWitness, label: &'static str) -> io::Result<Vec<u64>> {
    let mut offsets = Vec::with_capacity(witness.chunks.len());
    let mut next_offset = 0;

    for chunk in &witness.chunks {
        offsets.push(next_offset);
        next_offset = checked_add_u64(next_offset, chunk.byte_count, "witness chunk offset")?;
    }

    if next_offset != witness.byte_count {
        return Err(io_invalid_data(format!(
            "{label} witness chunk byte counts summed to {next_offset}, expected {}",
            witness.byte_count
        )));
    }

    Ok(offsets)
}

fn required_chunk_index(index: Option<usize>, label: &'static str) -> io::Result<usize> {
    index.ok_or_else(|| io_invalid_data(format!("reuse plan step was missing {label}")))
}

fn witness_chunk<'a>(
    chunks: &'a [JsonlChunkWitness],
    index: usize,
    label: &'static str,
) -> io::Result<&'a JsonlChunkWitness> {
    chunks.get(index).ok_or_else(|| {
        io_invalid_data(format!(
            "{label} witness chunk index {index} was out of range"
        ))
    })
}

fn validate_step_matches_chunk(
    step: &JsonlChunkReuseStep,
    chunk: &JsonlChunkWitness,
    label: &'static str,
) -> io::Result<()> {
    if step.start_line != chunk.start_line
        || step.line_count != chunk.line_count
        || step.byte_count != chunk.byte_count
    {
        return Err(io_invalid_data(format!(
            "reuse plan step metadata did not match {label} witness chunk {}",
            chunk.index
        )));
    }
    Ok(())
}

fn copy_witness_chunk<R: Read + Seek, W: Write>(
    reader: &mut R,
    offsets: &[u64],
    chunks: &[JsonlChunkWitness],
    index: usize,
    writer: &mut W,
    label: &'static str,
) -> io::Result<u64> {
    let chunk = witness_chunk(chunks, index, label)?;
    let offset = offsets.get(index).ok_or_else(|| {
        io_invalid_data(format!("{label} witness chunk offset {index} was missing"))
    })?;

    reader.seek(SeekFrom::Start(*offset))?;
    let mut limited = reader.take(chunk.byte_count);
    let copied = io::copy(&mut limited, writer)?;
    if copied != chunk.byte_count {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            format!(
                "{label} JSONL ended while copying chunk {index}: copied {copied} of {} bytes",
                chunk.byte_count
            ),
        ));
    }
    Ok(copied)
}

fn candidate_chunk_step(
    action: JsonlChunkReuseAction,
    base_index: Option<usize>,
    candidate_index: usize,
    chunk: &JsonlChunkWitness,
) -> JsonlChunkReuseStep {
    JsonlChunkReuseStep {
        action,
        base_index,
        candidate_index: Some(candidate_index),
        start_line: chunk.start_line,
        line_count: chunk.line_count,
        byte_count: chunk.byte_count,
    }
}

fn base_chunk_step(
    action: JsonlChunkReuseAction,
    base_index: usize,
    chunk: &JsonlChunkWitness,
) -> JsonlChunkReuseStep {
    JsonlChunkReuseStep {
        action,
        base_index: Some(base_index),
        candidate_index: None,
        start_line: chunk.start_line,
        line_count: chunk.line_count,
        byte_count: chunk.byte_count,
    }
}

fn build_reuse_schedule(actions: &[JsonlChunkReuseStep]) -> JsonlWitnessReuseSchedule {
    let mut schedule = JsonlWitnessReuseSchedule {
        total_actions: actions.len(),
        candidate_output_actions: 0,
        metadata_only_drop_actions: 0,
        reusable_actions: 0,
        rebuild_actions: 0,
        read_added_actions: 0,
        candidate_output_line_count: 0,
        candidate_output_byte_count: 0,
        reusable_byte_count: 0,
        rebuild_byte_count: 0,
        read_added_byte_count: 0,
        dropped_byte_count: 0,
        max_parallel_candidate_actions: 0,
        deterministic_candidate_order: candidate_actions_are_deterministically_ordered(actions),
    };

    for step in actions {
        match step.action {
            JsonlChunkReuseAction::ReuseUnchanged => {
                schedule.reusable_actions += 1;
                schedule.reusable_byte_count =
                    schedule.reusable_byte_count.saturating_add(step.byte_count);
                record_candidate_output(&mut schedule, step);
            }
            JsonlChunkReuseAction::RebuildCandidate => {
                schedule.rebuild_actions += 1;
                schedule.rebuild_byte_count =
                    schedule.rebuild_byte_count.saturating_add(step.byte_count);
                record_candidate_output(&mut schedule, step);
            }
            JsonlChunkReuseAction::ReadAdded => {
                schedule.read_added_actions += 1;
                schedule.read_added_byte_count = schedule
                    .read_added_byte_count
                    .saturating_add(step.byte_count);
                record_candidate_output(&mut schedule, step);
            }
            JsonlChunkReuseAction::DropRemoved => {
                schedule.metadata_only_drop_actions += 1;
                schedule.dropped_byte_count =
                    schedule.dropped_byte_count.saturating_add(step.byte_count);
            }
        }
    }

    schedule.max_parallel_candidate_actions = schedule.candidate_output_actions;
    schedule
}

fn record_candidate_output(schedule: &mut JsonlWitnessReuseSchedule, step: &JsonlChunkReuseStep) {
    schedule.candidate_output_actions += 1;
    schedule.candidate_output_line_count = schedule
        .candidate_output_line_count
        .saturating_add(step.line_count);
    schedule.candidate_output_byte_count = schedule
        .candidate_output_byte_count
        .saturating_add(step.byte_count);
}

fn candidate_actions_are_deterministically_ordered(actions: &[JsonlChunkReuseStep]) -> bool {
    let mut expected_candidate_index = 0;
    let mut drops_started = false;

    for step in actions {
        match step.candidate_index {
            Some(candidate_index)
                if !drops_started && candidate_index == expected_candidate_index =>
            {
                expected_candidate_index += 1;
            }
            Some(_) => return false,
            None => drops_started = true,
        }
    }

    true
}

fn work_batch_kind(step: &JsonlChunkReuseStep) -> JsonlWitnessWorkBatchKind {
    if step.candidate_index.is_some() {
        JsonlWitnessWorkBatchKind::CandidateOutput
    } else {
        JsonlWitnessWorkBatchKind::MetadataOnlyDrop
    }
}

fn push_work_batch(
    batches: &mut Vec<JsonlWitnessWorkBatch>,
    kind: JsonlWitnessWorkBatchKind,
    actions: Vec<JsonlChunkReuseStep>,
) {
    let index = batches.len();
    batches.push(build_work_batch(index, kind, actions));
}

fn build_work_batch(
    index: usize,
    kind: JsonlWitnessWorkBatchKind,
    actions: Vec<JsonlChunkReuseStep>,
) -> JsonlWitnessWorkBatch {
    let action_count = actions.len();
    let candidate_start_index = actions.iter().filter_map(|step| step.candidate_index).min();
    let candidate_end_index = actions
        .iter()
        .filter_map(|step| step.candidate_index)
        .max()
        .map(|index| index.saturating_add(1));
    let line_count = actions
        .iter()
        .fold(0_usize, |total, step| total.saturating_add(step.line_count));
    let byte_count = actions
        .iter()
        .fold(0_u64, |total, step| total.saturating_add(step.byte_count));

    JsonlWitnessWorkBatch {
        index,
        kind,
        action_count,
        candidate_start_index,
        candidate_end_index,
        line_count,
        byte_count,
        actions,
    }
}

fn work_batches_are_deterministically_ordered(batches: &[JsonlWitnessWorkBatch]) -> bool {
    let mut expected_candidate_index = 0;
    let mut drops_started = false;

    for (expected_batch_index, batch) in batches.iter().enumerate() {
        if batch.index != expected_batch_index {
            return false;
        }

        match batch.kind {
            JsonlWitnessWorkBatchKind::CandidateOutput if !drops_started => {
                if batch.candidate_start_index != Some(expected_candidate_index) {
                    return false;
                }
                expected_candidate_index += batch.action_count;
                if batch.candidate_end_index != Some(expected_candidate_index) {
                    return false;
                }
            }
            JsonlWitnessWorkBatchKind::CandidateOutput => return false,
            JsonlWitnessWorkBatchKind::MetadataOnlyDrop => drops_started = true,
        }
    }

    true
}

fn sum_chunk_bytes(chunks: &[JsonlChunkWitness]) -> u64 {
    chunks.iter().map(|chunk| chunk.byte_count).sum()
}

struct PendingChunk {
    index: usize,
    start_line: usize,
    line_count: usize,
    byte_count: u64,
    bytes: Vec<u8>,
    line_lengths: Vec<usize>,
}

impl PendingChunk {
    fn new(index: usize, start_line: usize, chunk_size_lines: usize) -> Self {
        Self {
            index,
            start_line,
            line_count: 0,
            byte_count: 0,
            bytes: Vec::new(),
            line_lengths: Vec::with_capacity(chunk_size_lines),
        }
    }

    fn is_full(&self, chunk_size_lines: usize) -> bool {
        self.line_count == chunk_size_lines
    }

    fn push_line(&mut self, line: &[u8], bytes_read: usize) -> io::Result<()> {
        self.line_count = self
            .line_count
            .checked_add(1)
            .ok_or_else(|| io_invalid_data("chunk line count overflowed usize"))?;
        self.byte_count = checked_add_bytes(self.byte_count, bytes_read, "chunk byte count")?;
        self.line_lengths.push(bytes_read);
        self.bytes.extend_from_slice(line);
        Ok(())
    }

    fn finish(self) -> io::Result<JsonlChunkWitness> {
        let mut hasher = Sha256::new();
        hasher.update(CHUNK_DOMAIN);
        hasher.update(index_to_bytes(self.index)?);
        hasher.update(index_to_bytes(self.start_line)?);

        let mut first_line_hash = None;
        let mut last_line_hash = None;
        let mut offset = 0usize;

        for (line_index, bytes_read) in self.line_lengths.iter().copied().enumerate() {
            let next_offset = offset
                .checked_add(bytes_read)
                .ok_or_else(|| io_invalid_data("chunk byte offset overflowed usize"))?;
            let line = self
                .bytes
                .get(offset..next_offset)
                .ok_or_else(|| io_invalid_data("chunk line range exceeded buffered bytes"))?;
            let line_hash = sha256_hex(line);

            if first_line_hash.is_none() {
                first_line_hash = Some(line_hash.clone());
            }
            last_line_hash = Some(line_hash);

            hasher.update(index_to_bytes(line_index)?);
            hasher.update(length_to_bytes(bytes_read)?);
            hasher.update(line);
            offset = next_offset;
        }

        Ok(JsonlChunkWitness {
            index: self.index,
            start_line: self.start_line,
            line_count: self.line_count,
            byte_count: self.byte_count,
            hash: hex_encode(&hasher.finalize()),
            first_line_hash,
            last_line_hash,
        })
    }
}

fn build_parallel_chunks<R: BufRead>(
    mut reader: R,
    chunk_size_lines: usize,
    max_parallelism: usize,
) -> io::Result<(Vec<JsonlChunkWitness>, usize, u64)> {
    thread::scope(|scope| {
        let (result_sender, result_receiver) =
            mpsc::channel::<(usize, io::Result<JsonlChunkWitness>)>();
        let mut task_senders = Vec::with_capacity(max_parallelism);
        let mut handles = Vec::with_capacity(max_parallelism);

        for _ in 0..max_parallelism {
            let (task_sender, task_receiver) = mpsc::sync_channel::<PendingChunk>(1);
            let result_sender = result_sender.clone();
            task_senders.push(task_sender);
            handles.push(scope.spawn(move || {
                while let Ok(chunk) = task_receiver.recv() {
                    let index = chunk.index;
                    if result_sender.send((index, chunk.finish())).is_err() {
                        break;
                    }
                }
            }));
        }
        drop(result_sender);

        let mut line_count = 0usize;
        let mut byte_count = 0u64;
        let mut chunk_count = 0usize;
        let mut next_worker = 0usize;
        let mut current_chunk = PendingChunk::new(0, 0, chunk_size_lines);
        let mut line = Vec::new();

        loop {
            line.clear();
            let bytes_read = reader.read_until(b'\n', &mut line)?;
            if bytes_read == 0 {
                break;
            }

            if current_chunk.is_full(chunk_size_lines) {
                send_pending_chunk(&task_senders, &mut next_worker, current_chunk)?;
                chunk_count = chunk_count
                    .checked_add(1)
                    .ok_or_else(|| io_invalid_data("JSONL chunk count overflowed usize"))?;
                current_chunk = PendingChunk::new(chunk_count, line_count, chunk_size_lines);
            }

            let next_line_count = line_count
                .checked_add(1)
                .ok_or_else(|| io_invalid_data("JSONL line count overflowed usize"))?;
            let next_byte_count = checked_add_bytes(byte_count, bytes_read, "JSONL byte count")?;
            current_chunk.push_line(&line, bytes_read)?;
            line_count = next_line_count;
            byte_count = next_byte_count;
        }

        if current_chunk.line_count > 0 {
            send_pending_chunk(&task_senders, &mut next_worker, current_chunk)?;
            chunk_count = chunk_count
                .checked_add(1)
                .ok_or_else(|| io_invalid_data("JSONL chunk count overflowed usize"))?;
        }
        drop(task_senders);

        let chunks = collect_parallel_chunks(&result_receiver, chunk_count)?;

        for handle in handles {
            handle
                .join()
                .map_err(|_| io::Error::other("parallel JSONL witness worker panicked"))?;
        }

        Ok((chunks, line_count, byte_count))
    })
}

fn send_pending_chunk(
    task_senders: &[mpsc::SyncSender<PendingChunk>],
    next_worker: &mut usize,
    chunk: PendingChunk,
) -> io::Result<()> {
    let sender = task_senders
        .get(*next_worker)
        .ok_or_else(|| io_invalid_data("parallel JSONL witness worker set was empty"))?;
    *next_worker = (*next_worker + 1) % task_senders.len();
    sender.send(chunk).map_err(|_| {
        io::Error::new(
            io::ErrorKind::BrokenPipe,
            "parallel JSONL witness worker disconnected",
        )
    })
}

fn collect_parallel_chunks(
    result_receiver: &mpsc::Receiver<(usize, io::Result<JsonlChunkWitness>)>,
    chunk_count: usize,
) -> io::Result<Vec<JsonlChunkWitness>> {
    let mut chunks_by_index = vec![None; chunk_count];

    for _ in 0..chunk_count {
        let (index, chunk) = result_receiver.recv().map_err(|_| {
            io::Error::new(
                io::ErrorKind::BrokenPipe,
                "parallel JSONL witness worker result channel closed",
            )
        })?;
        let slot = chunks_by_index
            .get_mut(index)
            .ok_or_else(|| io_invalid_data("parallel JSONL witness chunk index out of range"))?;
        if slot.replace(chunk?).is_some() {
            return Err(io_invalid_data(
                "parallel JSONL witness duplicate chunk index",
            ));
        }
    }

    chunks_by_index
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            chunk.ok_or_else(|| {
                io_invalid_data(format!(
                    "parallel JSONL witness missing chunk index {index}"
                ))
            })
        })
        .collect()
}

struct ChunkBuilder {
    index: usize,
    start_line: usize,
    line_count: usize,
    byte_count: u64,
    hasher: Sha256,
    first_line_hash: Option<String>,
    last_line_hash: Option<String>,
}

impl ChunkBuilder {
    fn new(index: usize, start_line: usize) -> io::Result<Self> {
        let mut hasher = Sha256::new();
        hasher.update(CHUNK_DOMAIN);
        hasher.update(index_to_bytes(index)?);
        hasher.update(index_to_bytes(start_line)?);

        Ok(Self {
            index,
            start_line,
            line_count: 0,
            byte_count: 0,
            hasher,
            first_line_hash: None,
            last_line_hash: None,
        })
    }

    fn push_line(&mut self, line: &[u8], bytes_read: usize) -> io::Result<()> {
        let line_hash = sha256_hex(line);

        if self.first_line_hash.is_none() {
            self.first_line_hash = Some(line_hash.clone());
        }
        self.last_line_hash = Some(line_hash);

        self.hasher.update(index_to_bytes(self.line_count)?);
        self.hasher.update(length_to_bytes(bytes_read)?);
        self.hasher.update(line);
        self.line_count = self
            .line_count
            .checked_add(1)
            .ok_or_else(|| io_invalid_data("chunk line count overflowed usize"))?;
        self.byte_count = checked_add_bytes(self.byte_count, bytes_read, "chunk byte count")?;
        Ok(())
    }

    fn finish(self) -> JsonlChunkWitness {
        let hash = hex_encode(&self.hasher.finalize());

        JsonlChunkWitness {
            index: self.index,
            start_line: self.start_line,
            line_count: self.line_count,
            byte_count: self.byte_count,
            hash,
            first_line_hash: self.first_line_hash,
            last_line_hash: self.last_line_hash,
        }
    }
}

fn compute_root_hash(
    chunk_size_lines: usize,
    line_count: usize,
    byte_count: u64,
    chunks: &[JsonlChunkWitness],
) -> io::Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(ROOT_DOMAIN);
    hash_field(&mut hasher, JSONL_WITNESS_SCHEMA_VERSION.as_bytes())?;
    hasher.update(index_to_bytes(chunk_size_lines)?);
    hasher.update(index_to_bytes(line_count)?);
    hasher.update(byte_count.to_le_bytes());
    hasher.update(index_to_bytes(chunks.len())?);

    for chunk in chunks {
        hasher.update(index_to_bytes(chunk.index)?);
        hasher.update(index_to_bytes(chunk.start_line)?);
        hasher.update(index_to_bytes(chunk.line_count)?);
        hasher.update(chunk.byte_count.to_le_bytes());
        hash_field(&mut hasher, chunk.hash.as_bytes())?;
        hash_optional_field(&mut hasher, chunk.first_line_hash.as_deref())?;
        hash_optional_field(&mut hasher, chunk.last_line_hash.as_deref())?;
    }

    Ok(hex_encode(&hasher.finalize()))
}

fn hash_field(hasher: &mut Sha256, bytes: &[u8]) -> io::Result<()> {
    hasher.update(length_to_bytes(bytes.len())?);
    hasher.update(FIELD_SEPARATOR);
    hasher.update(bytes);
    hasher.update(FIELD_SEPARATOR);
    Ok(())
}

fn hash_optional_field(hasher: &mut Sha256, value: Option<&str>) -> io::Result<()> {
    match value {
        Some(value) => {
            hasher.update([1]);
            hash_field(hasher, value.as_bytes())?;
        }
        None => hasher.update([0]),
    }
    Ok(())
}

fn checked_add_bytes(total: u64, delta: usize, label: &'static str) -> io::Result<u64> {
    let delta = length_to_u64(delta)?;
    checked_add_u64(total, delta, label)
}

fn checked_add_u64(total: u64, delta: u64, label: &'static str) -> io::Result<u64> {
    total
        .checked_add(delta)
        .ok_or_else(|| io_invalid_data(format!("{label} overflowed u64")))
}

fn index_to_bytes(value: usize) -> io::Result<[u8; 8]> {
    length_to_bytes(value)
}

fn length_to_bytes(value: usize) -> io::Result<[u8; 8]> {
    Ok(length_to_u64(value)?.to_le_bytes())
}

fn length_to_u64(value: usize) -> io::Result<u64> {
    u64::try_from(value).map_err(|_| io_invalid_data("length exceeded u64"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

fn io_invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn witness(input: &[u8], chunk_size_lines: usize) -> JsonlMerkleWitness {
        build_jsonl_merkle_witness(Cursor::new(input), chunk_size_lines).unwrap()
    }

    fn materialize_plan(
        base_input: &[u8],
        candidate_input: &[u8],
        base: &JsonlMerkleWitness,
        candidate: &JsonlMerkleWitness,
        plan: &JsonlWitnessReusePlan,
    ) -> io::Result<(JsonlWitnessReuseMaterialization, Vec<u8>)> {
        let mut base_reader = Cursor::new(base_input);
        let mut candidate_reader = Cursor::new(candidate_input);
        let mut output = Vec::new();
        let summary = materialize_jsonl_witness_reuse_plan(
            &mut base_reader,
            &mut candidate_reader,
            &mut output,
            base,
            candidate,
            plan,
        )?;
        Ok((summary, output))
    }

    #[test]
    fn builds_deterministic_chunk_witnesses() {
        let input = b"{\"id\":\"a\"}\n{\"id\":\"b\"}\n{\"id\":\"c\"}\n";

        let first = build_jsonl_merkle_witness(Cursor::new(input), 2).unwrap();
        let second = build_jsonl_merkle_witness(Cursor::new(input), 2).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.schema_version, JSONL_WITNESS_SCHEMA_VERSION);
        assert_eq!(first.chunk_size_lines, 2);
        assert_eq!(first.line_count, 3);
        assert_eq!(first.byte_count, u64::try_from(input.len()).unwrap());
        assert_eq!(first.chunks.len(), 2);
        assert_eq!(first.chunks[0].index, 0);
        assert_eq!(first.chunks[0].start_line, 0);
        assert_eq!(first.chunks[0].line_count, 2);
        assert_eq!(first.chunks[1].index, 1);
        assert_eq!(first.chunks[1].start_line, 2);
        assert_eq!(first.chunks[1].line_count, 1);
        assert_eq!(
            first.chunks[0].first_line_hash.as_deref(),
            Some(sha256_hex(b"{\"id\":\"a\"}\n").as_str())
        );
        assert_eq!(
            first.chunks[1].last_line_hash.as_deref(),
            Some(sha256_hex(b"{\"id\":\"c\"}\n").as_str())
        );
    }

    #[test]
    fn parallel_witness_matches_serial_witness() {
        let input =
            b"{\"id\":\"a\"}\n{\"id\":\"b\"}\n{\"id\":\"c\"}\n{\"id\":\"d\"}\n{\"id\":\"e\"}";
        let serial = build_jsonl_merkle_witness(Cursor::new(input), 2).unwrap();

        let parallel = build_jsonl_merkle_witness_parallel(Cursor::new(input), 2, 3).unwrap();

        assert_eq!(parallel, serial);
    }

    #[test]
    fn parallel_witness_parallelism_one_uses_serial_output() {
        let input = b"a\nb\nc\n";
        let serial = build_jsonl_merkle_witness(Cursor::new(input), 1).unwrap();

        let parallel = build_jsonl_merkle_witness_parallel(Cursor::new(input), 1, 1).unwrap();

        assert_eq!(parallel, serial);
    }

    #[test]
    fn parallel_witness_rejects_zero_parallelism() {
        let err = build_jsonl_merkle_witness_parallel(Cursor::new(b"a\n"), 1, 0).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn root_changes_when_a_byte_changes() {
        let original = build_jsonl_merkle_witness(Cursor::new(b"{\"id\":\"a\"}\n"), 2).unwrap();
        let changed = build_jsonl_merkle_witness(Cursor::new(b"{\"id\":\"b\"}\n"), 2).unwrap();

        assert_ne!(original.root_hash, changed.root_hash);
        assert_ne!(original.chunks[0].hash, changed.chunks[0].hash);
    }

    #[test]
    fn root_changes_when_line_order_changes() {
        let first = build_jsonl_merkle_witness(Cursor::new(b"a\nb\n"), 2).unwrap();
        let second = build_jsonl_merkle_witness(Cursor::new(b"b\na\n"), 2).unwrap();

        assert_ne!(first.root_hash, second.root_hash);
    }

    #[test]
    fn final_line_without_newline_is_counted_and_hashed() {
        let witness = build_jsonl_merkle_witness(Cursor::new(b"a\nb"), 1).unwrap();

        assert_eq!(witness.line_count, 2);
        assert_eq!(witness.byte_count, 3);
        assert_eq!(witness.chunks.len(), 2);
        assert_eq!(
            witness.chunks[1].first_line_hash.as_deref(),
            Some(sha256_hex(b"b").as_str())
        );
        assert_eq!(
            witness.chunks[1].last_line_hash.as_deref(),
            Some(sha256_hex(b"b").as_str())
        );
    }

    #[test]
    fn zero_chunk_size_is_rejected() {
        let err = build_jsonl_merkle_witness(Cursor::new(b"a\n"), 0).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn empty_input_has_stable_empty_root() {
        let first = build_jsonl_merkle_witness(Cursor::new(Vec::new()), 2).unwrap();
        let second = build_jsonl_merkle_witness(Cursor::new(Vec::new()), 2).unwrap();

        assert_eq!(first, second);
        assert_eq!(first.line_count, 0);
        assert_eq!(first.byte_count, 0);
        assert!(first.chunks.is_empty());
        assert!(!first.root_hash.is_empty());
    }

    #[test]
    fn comparison_reports_identical_witnesses_as_reusable() {
        let base = witness(b"a\nb\nc\n", 2);
        let candidate = witness(b"a\nb\nc\n", 2);

        let comparison = compare_jsonl_merkle_witnesses(&base, &candidate);

        assert!(comparison.schema_versions_match);
        assert!(comparison.chunk_size_lines_match);
        assert!(comparison.root_hashes_match);
        assert!(!comparison.drift_detected);
        assert_eq!(comparison.comparable_chunk_count, 2);
        assert_eq!(comparison.unchanged_chunks, 2);
        assert_eq!(comparison.changed_chunks, 0);
        assert_eq!(comparison.added_chunks, 0);
        assert_eq!(comparison.removed_chunks, 0);
        assert_eq!(comparison.unchanged_byte_count, candidate.byte_count);
        assert_eq!(comparison.safe_reuse_prefix_chunks, 2);
        assert_eq!(comparison.first_changed_chunk_index, None);
    }

    #[test]
    fn comparison_localizes_changed_chunk() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\nB\nc\n", 1);

        let comparison = compare_jsonl_merkle_witnesses(&base, &candidate);

        assert!(comparison.drift_detected);
        assert!(!comparison.root_hashes_match);
        assert_eq!(comparison.comparable_chunk_count, 3);
        assert_eq!(comparison.unchanged_chunks, 2);
        assert_eq!(comparison.changed_chunks, 1);
        assert_eq!(comparison.added_chunks, 0);
        assert_eq!(comparison.removed_chunks, 0);
        assert_eq!(comparison.safe_reuse_prefix_chunks, 1);
        assert_eq!(comparison.first_changed_chunk_index, Some(1));
        assert_eq!(
            comparison.changed_base_byte_count,
            base.chunks[1].byte_count
        );
        assert_eq!(
            comparison.changed_candidate_byte_count,
            candidate.chunks[1].byte_count
        );
    }

    #[test]
    fn comparison_reports_tail_appends_without_marking_prefix_changed() {
        let base = witness(b"a\nb\n", 1);
        let candidate = witness(b"a\nb\nc\n", 1);

        let comparison = compare_jsonl_merkle_witnesses(&base, &candidate);

        assert!(comparison.drift_detected);
        assert_eq!(comparison.comparable_chunk_count, 2);
        assert_eq!(comparison.unchanged_chunks, 2);
        assert_eq!(comparison.changed_chunks, 0);
        assert_eq!(comparison.added_chunks, 1);
        assert_eq!(comparison.removed_chunks, 0);
        assert_eq!(comparison.added_byte_count, candidate.chunks[2].byte_count);
        assert_eq!(comparison.removed_byte_count, 0);
        assert_eq!(comparison.safe_reuse_prefix_chunks, 2);
        assert_eq!(comparison.first_changed_chunk_index, Some(2));
    }

    #[test]
    fn comparison_rejects_incompatible_chunk_sizes_for_reuse() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\nb\nc\n", 2);

        let comparison = compare_jsonl_merkle_witnesses(&base, &candidate);

        assert!(comparison.drift_detected);
        assert!(!comparison.chunk_size_lines_match);
        assert_eq!(comparison.comparable_chunk_count, 0);
        assert_eq!(comparison.unchanged_chunks, 0);
        assert_eq!(comparison.changed_chunks, 0);
        assert_eq!(comparison.added_chunks, candidate.chunks.len());
        assert_eq!(comparison.removed_chunks, base.chunks.len());
        assert_eq!(comparison.safe_reuse_prefix_chunks, 0);
        assert_eq!(comparison.first_changed_chunk_index, Some(0));
    }

    #[test]
    fn reuse_plan_reuses_identical_candidate_chunks_in_order() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\nb\nc\n", 1);

        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let actions: Vec<_> = plan.actions.iter().map(|step| step.action).collect();
        assert_eq!(
            actions,
            vec![
                JsonlChunkReuseAction::ReuseUnchanged,
                JsonlChunkReuseAction::ReuseUnchanged,
                JsonlChunkReuseAction::ReuseUnchanged,
            ]
        );
        assert!(
            plan.actions
                .iter()
                .enumerate()
                .all(|(index, step)| step.candidate_index == Some(index))
        );
        assert_eq!(plan.comparison.safe_reuse_prefix_chunks, 3);
    }

    #[test]
    fn reuse_plan_separates_changed_and_added_candidate_chunks() {
        let base = witness(b"a\nb\n", 1);
        let candidate = witness(b"a\nB\nc\n", 1);

        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let actions: Vec<_> = plan.actions.iter().map(|step| step.action).collect();
        assert_eq!(
            actions,
            vec![
                JsonlChunkReuseAction::ReuseUnchanged,
                JsonlChunkReuseAction::RebuildCandidate,
                JsonlChunkReuseAction::ReadAdded,
            ]
        );
        assert_eq!(plan.actions[1].base_index, Some(1));
        assert_eq!(plan.actions[1].candidate_index, Some(1));
        assert_eq!(plan.actions[2].base_index, None);
        assert_eq!(plan.actions[2].candidate_index, Some(2));
        assert_eq!(plan.schedule.total_actions, 3);
        assert_eq!(plan.schedule.candidate_output_actions, 3);
        assert_eq!(plan.schedule.metadata_only_drop_actions, 0);
        assert_eq!(plan.schedule.reusable_actions, 1);
        assert_eq!(plan.schedule.rebuild_actions, 1);
        assert_eq!(plan.schedule.read_added_actions, 1);
        assert_eq!(plan.schedule.max_parallel_candidate_actions, 3);
        assert!(plan.schedule.deterministic_candidate_order);
        assert_eq!(
            plan.schedule.candidate_output_line_count,
            candidate.line_count
        );
        assert_eq!(
            plan.schedule.candidate_output_byte_count,
            candidate.byte_count
        );
        assert_eq!(
            plan.schedule.reusable_byte_count,
            candidate.chunks[0].byte_count
        );
        assert_eq!(
            plan.schedule.rebuild_byte_count,
            candidate.chunks[1].byte_count
        );
        assert_eq!(
            plan.schedule.read_added_byte_count,
            candidate.chunks[2].byte_count
        );
    }

    #[test]
    fn reuse_plan_appends_removed_base_chunks_after_candidate_actions() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\n", 1);

        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let actions: Vec<_> = plan.actions.iter().map(|step| step.action).collect();
        assert_eq!(
            actions,
            vec![
                JsonlChunkReuseAction::ReuseUnchanged,
                JsonlChunkReuseAction::DropRemoved,
                JsonlChunkReuseAction::DropRemoved,
            ]
        );
        assert_eq!(plan.actions[0].candidate_index, Some(0));
        assert_eq!(plan.actions[1].base_index, Some(1));
        assert_eq!(plan.actions[1].candidate_index, None);
        assert_eq!(plan.actions[2].base_index, Some(2));
        assert_eq!(plan.schedule.candidate_output_actions, 1);
        assert_eq!(plan.schedule.metadata_only_drop_actions, 2);
        assert_eq!(plan.schedule.max_parallel_candidate_actions, 1);
        assert_eq!(
            plan.schedule.dropped_byte_count,
            base.chunks[1].byte_count + base.chunks[2].byte_count
        );
        assert!(plan.schedule.deterministic_candidate_order);
    }

    #[test]
    fn reuse_plan_rebuilds_all_candidate_chunks_when_chunk_sizes_differ() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\nb\nc\n", 2);

        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let actions: Vec<_> = plan.actions.iter().map(|step| step.action).collect();
        assert_eq!(
            actions,
            vec![
                JsonlChunkReuseAction::RebuildCandidate,
                JsonlChunkReuseAction::RebuildCandidate,
                JsonlChunkReuseAction::DropRemoved,
                JsonlChunkReuseAction::DropRemoved,
                JsonlChunkReuseAction::DropRemoved,
            ]
        );
        assert!(plan.actions[0].base_index.is_none());
        assert_eq!(plan.actions[0].candidate_index, Some(0));
        assert_eq!(plan.actions[2].base_index, Some(0));
        assert_eq!(plan.comparison.comparable_chunk_count, 0);
    }

    #[test]
    fn reuse_materialization_reconstructs_candidate_bytes_from_mixed_actions() {
        let base_input: &[u8] = b"a\nb\nc\n";
        let candidate_input: &[u8] = b"a\nB\nc\nd\n";
        let base = witness(base_input, 1);
        let candidate = witness(candidate_input, 1);
        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let (summary, output) =
            materialize_plan(base_input, candidate_input, &base, &candidate, &plan).unwrap();

        assert_eq!(output, candidate_input);
        assert_eq!(summary.reused_chunks, 2);
        assert_eq!(summary.rebuilt_chunks, 1);
        assert_eq!(summary.read_added_chunks, 1);
        assert_eq!(summary.dropped_chunks, 0);
        assert_eq!(summary.output_byte_count, candidate.byte_count);
        assert_eq!(
            summary.reused_byte_count,
            candidate.chunks[0].byte_count + candidate.chunks[2].byte_count
        );
        assert_eq!(summary.rebuilt_byte_count, candidate.chunks[1].byte_count);
        assert_eq!(
            summary.read_added_byte_count,
            candidate.chunks[3].byte_count
        );
    }

    #[test]
    fn reuse_materialization_counts_removed_base_chunks_without_emitting_them() {
        let base_input: &[u8] = b"a\nb\nc\n";
        let candidate_input: &[u8] = b"a\n";
        let base = witness(base_input, 1);
        let candidate = witness(candidate_input, 1);
        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let (summary, output) =
            materialize_plan(base_input, candidate_input, &base, &candidate, &plan).unwrap();

        assert_eq!(output, candidate_input);
        assert_eq!(summary.reused_chunks, 1);
        assert_eq!(summary.dropped_chunks, 2);
        assert_eq!(summary.output_byte_count, candidate.byte_count);
        assert_eq!(
            summary.dropped_byte_count,
            base.chunks[1].byte_count + base.chunks[2].byte_count
        );
    }

    #[test]
    fn reuse_materialization_rebuilds_candidate_when_chunk_sizes_differ() {
        let base_input: &[u8] = b"a\nb\nc\n";
        let candidate_input: &[u8] = b"a\nb\nc\n";
        let base = witness(base_input, 1);
        let candidate = witness(candidate_input, 2);
        let plan = plan_jsonl_witness_reuse(&base, &candidate);

        let (summary, output) =
            materialize_plan(base_input, candidate_input, &base, &candidate, &plan).unwrap();

        assert_eq!(output, candidate_input);
        assert_eq!(summary.reused_chunks, 0);
        assert_eq!(summary.rebuilt_chunks, candidate.chunks.len());
        assert_eq!(summary.dropped_chunks, base.chunks.len());
        assert_eq!(summary.output_byte_count, candidate.byte_count);
    }

    #[test]
    fn reuse_materialization_rejects_tampered_reuse_action() {
        let base_input: &[u8] = b"a\n";
        let candidate_input: &[u8] = b"A\n";
        let base = witness(base_input, 1);
        let candidate = witness(candidate_input, 1);
        let mut plan = plan_jsonl_witness_reuse(&base, &candidate);
        for step in &mut plan.actions {
            step.action = JsonlChunkReuseAction::ReuseUnchanged;
        }
        plan.schedule = build_reuse_schedule(&plan.actions);

        let err =
            materialize_plan(base_input, candidate_input, &base, &candidate, &plan).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert!(err.to_string().contains("no longer matches"));
    }

    #[test]
    fn parallel_work_plan_batches_candidate_actions_before_metadata_drops() {
        let base = witness(b"a\nb\nc\nd\n", 1);
        let candidate = witness(b"a\nB\nc\n", 1);
        let reuse_plan = plan_jsonl_witness_reuse(&base, &candidate);

        let work_plan = plan_jsonl_witness_parallel_work(&reuse_plan, 2).unwrap();

        assert_eq!(work_plan.max_parallelism, 2);
        assert_eq!(work_plan.total_batches, 3);
        assert_eq!(work_plan.candidate_output_batches, 2);
        assert_eq!(work_plan.metadata_only_drop_batches, 1);
        assert!(work_plan.deterministic_batch_order);

        assert_eq!(
            work_plan.batches[0].kind,
            JsonlWitnessWorkBatchKind::CandidateOutput
        );
        assert_eq!(work_plan.batches[0].action_count, 2);
        assert_eq!(work_plan.batches[0].candidate_start_index, Some(0));
        assert_eq!(work_plan.batches[0].candidate_end_index, Some(2));
        assert_eq!(
            work_plan.batches[0]
                .actions
                .iter()
                .map(|step| step.action)
                .collect::<Vec<_>>(),
            vec![
                JsonlChunkReuseAction::ReuseUnchanged,
                JsonlChunkReuseAction::RebuildCandidate,
            ]
        );

        assert_eq!(
            work_plan.batches[1].kind,
            JsonlWitnessWorkBatchKind::CandidateOutput
        );
        assert_eq!(work_plan.batches[1].action_count, 1);
        assert_eq!(work_plan.batches[1].candidate_start_index, Some(2));
        assert_eq!(work_plan.batches[1].candidate_end_index, Some(3));
        assert_eq!(
            work_plan.batches[1].line_count,
            candidate.chunks[2].line_count
        );
        assert_eq!(
            work_plan.batches[1].byte_count,
            candidate.chunks[2].byte_count
        );

        assert_eq!(
            work_plan.batches[2].kind,
            JsonlWitnessWorkBatchKind::MetadataOnlyDrop
        );
        assert_eq!(work_plan.batches[2].action_count, 1);
        assert_eq!(work_plan.batches[2].candidate_start_index, None);
        assert_eq!(work_plan.batches[2].candidate_end_index, None);
        assert_eq!(work_plan.batches[2].line_count, base.chunks[3].line_count);
        assert_eq!(work_plan.batches[2].byte_count, base.chunks[3].byte_count);
    }

    #[test]
    fn parallel_work_plan_rejects_zero_parallelism() {
        let base = witness(b"a\n", 1);
        let candidate = witness(b"a\n", 1);
        let reuse_plan = plan_jsonl_witness_reuse(&base, &candidate);

        let err = plan_jsonl_witness_parallel_work(&reuse_plan, 0).unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn parallel_work_plan_rebuilds_incompatible_candidate_chunks_in_waves() {
        let base = witness(b"a\nb\nc\n", 1);
        let candidate = witness(b"a\nb\nc\n", 2);
        let reuse_plan = plan_jsonl_witness_reuse(&base, &candidate);

        let work_plan = plan_jsonl_witness_parallel_work(&reuse_plan, 1).unwrap();

        assert_eq!(work_plan.total_batches, 5);
        assert_eq!(work_plan.candidate_output_batches, 2);
        assert_eq!(work_plan.metadata_only_drop_batches, 3);
        assert!(work_plan.deterministic_batch_order);
        assert_eq!(
            work_plan
                .batches
                .iter()
                .map(|batch| batch.kind)
                .collect::<Vec<_>>(),
            vec![
                JsonlWitnessWorkBatchKind::CandidateOutput,
                JsonlWitnessWorkBatchKind::CandidateOutput,
                JsonlWitnessWorkBatchKind::MetadataOnlyDrop,
                JsonlWitnessWorkBatchKind::MetadataOnlyDrop,
                JsonlWitnessWorkBatchKind::MetadataOnlyDrop,
            ]
        );
        assert_eq!(work_plan.batches[0].candidate_start_index, Some(0));
        assert_eq!(work_plan.batches[1].candidate_start_index, Some(1));
    }
}
