//! `SQLite` storage implementation.

use crate::error::{BeadsError, Result};
use crate::format::{IssueDetails, IssueWithDependencyMetadata};
use crate::model::{Comment, Event, EventType, Issue, IssueType, Priority, Status};
use crate::storage::events::get_events;
use crate::storage::schema::apply_schema;
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Transaction};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::Path;

/// SQLite-based storage backend.
#[derive(Debug)]
pub struct SqliteStorage {
    conn: Connection,
}

/// Context for a mutation operation, tracking side effects.
pub struct MutationContext {
    pub op_name: String,
    pub actor: String,
    pub events: Vec<Event>,
    pub dirty_ids: HashSet<String>,
    pub invalidate_blocked_cache: bool,
}

impl MutationContext {
    #[must_use]
    pub fn new(op_name: &str, actor: &str) -> Self {
        Self {
            op_name: op_name.to_string(),
            actor: actor.to_string(),
            events: Vec::new(),
            dirty_ids: HashSet::new(),
            invalidate_blocked_cache: false,
        }
    }

    pub fn record_event(&mut self, event_type: EventType, issue_id: &str, details: Option<String>) {
        self.events.push(Event {
            id: 0, // Placeholder, DB assigns auto-inc ID
            issue_id: issue_id.to_string(),
            event_type,
            actor: self.actor.clone(),
            old_value: None,
            new_value: None,
            comment: details,
            created_at: Utc::now(),
        });
    }

    /// Record a field change event with old and new values.
    pub fn record_field_change(
        &mut self,
        event_type: EventType,
        issue_id: &str,
        old_value: Option<String>,
        new_value: Option<String>,
        comment: Option<String>,
    ) {
        self.events.push(Event {
            id: 0,
            issue_id: issue_id.to_string(),
            event_type,
            actor: self.actor.clone(),
            old_value,
            new_value,
            comment,
            created_at: Utc::now(),
        });
    }

    pub fn mark_dirty(&mut self, issue_id: &str) {
        self.dirty_ids.insert(issue_id.to_string());
    }

    pub const fn invalidate_cache(&mut self) {
        self.invalidate_blocked_cache = true;
    }
}

impl SqliteStorage {
    /// Open a new connection to the database at the given path.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established or schema application fails.
    pub fn open(path: &Path) -> Result<Self> {
        let conn = Connection::open(path)?;
        apply_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Open an in-memory database for testing.
    ///
    /// # Errors
    ///
    /// Returns an error if the connection cannot be established.
    pub fn open_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        apply_schema(&conn)?;
        Ok(Self { conn })
    }

    /// Execute a mutation with the 4-step transaction protocol.
    ///
    /// # Errors
    ///
    /// Returns an error if any step fails (e.g. database error, logic error).
    /// The transaction is rolled back on error.
    pub fn mutate<F, R>(&mut self, op: &str, actor: &str, f: F) -> Result<R>
    where
        F: FnOnce(&Transaction, &mut MutationContext) -> Result<R>,
    {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let mut ctx = MutationContext::new(op, actor);

        let result = f(&tx, &mut ctx)?;

        // Write events
        for event in ctx.events {
            tx.execute(
                "INSERT INTO events (issue_id, event_type, actor, old_value, new_value, comment, created_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    event.issue_id,
                    event.event_type.as_str(),
                    event.actor,
                    event.old_value,
                    event.new_value,
                    event.comment,
                    event.created_at.to_rfc3339()
                ],
            )?;
        }

        // Mark dirty
        for id in ctx.dirty_ids {
            tx.execute(
                "INSERT OR REPLACE INTO dirty_issues (issue_id, marked_at) VALUES (?, ?)",
                rusqlite::params![id, Utc::now().to_rfc3339()],
            )?;
        }

        // Invalidate cache
        if ctx.invalidate_blocked_cache {
            tx.execute("DELETE FROM blocked_issues_cache", [])?;
        }

        tx.commit()?;
        Ok(result)
    }

    /// Create a new issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue cannot be inserted (e.g. ID collision).
    pub fn create_issue(&mut self, issue: &Issue, actor: &str) -> Result<()> {
        self.mutate("create_issue", actor, |tx, ctx| {
            tx.execute(
                "INSERT INTO issues (
                    id, title, description, status, priority, issue_type, 
                    assignee, owner, estimated_minutes, 
                    created_at, created_by, updated_at, 
                    due_at, defer_until, external_ref
                ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    issue.id,
                    issue.title,
                    issue.description,
                    issue.status.as_str(),
                    issue.priority.0,
                    issue.issue_type.as_str(),
                    issue.assignee,
                    issue.owner,
                    issue.estimated_minutes,
                    issue.created_at.to_rfc3339(),
                    issue.created_by,
                    issue.updated_at.to_rfc3339(),
                    issue.due_at.map(|t| t.to_rfc3339()),
                    issue.defer_until.map(|t| t.to_rfc3339()),
                    issue.external_ref,
                ],
            )?;

            ctx.record_event(
                EventType::Created,
                &issue.id,
                Some(format!("Created issue: {}", issue.title)),
            );

            ctx.mark_dirty(&issue.id);

            Ok(())
        })
    }

    /// Update an issue's fields.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue doesn't exist or the update fails.
    pub fn update_issue(&mut self, id: &str, updates: &IssueUpdate, actor: &str) -> Result<Issue> {
        let existing = self
            .get_issue(id)?
            .ok_or_else(|| BeadsError::IssueNotFound { id: id.to_string() })?;

        if updates.is_empty() {
            return Ok(existing);
        }

        self.mutate("update_issue", actor, |tx, ctx| {
            let mut set_clauses: Vec<String> = vec![];
            let mut params: Vec<Box<dyn rusqlite::ToSql>> = vec![];

            // Helper to add update
            let mut add_update = |field: &str, val: Box<dyn rusqlite::ToSql>| {
                set_clauses.push(format!("{field} = ?"));
                params.push(val);
            };

            // Title
            if let Some(ref title) = updates.title {
                add_update("title", Box::new(title.clone()));
                ctx.record_field_change(
                    EventType::Updated,
                    id,
                    Some(existing.title.clone()),
                    Some(title.clone()),
                    Some("Title changed".to_string()),
                );
            }

            // Simple text fields
            if let Some(ref val) = updates.description {
                add_update("description", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.design {
                add_update("design", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.acceptance_criteria {
                add_update("acceptance_criteria", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.notes {
                add_update("notes", Box::new(val.clone()));
            }

            // Status
            if let Some(ref status) = updates.status {
                let old_status = existing.status.as_str().to_string();
                add_update("status", Box::new(status.as_str().to_string()));
                ctx.record_field_change(
                    EventType::StatusChanged,
                    id,
                    Some(old_status),
                    Some(status.as_str().to_string()),
                    None,
                );
                ctx.invalidate_cache();
            }

            // Priority
            if let Some(priority) = updates.priority {
                let old_priority = existing.priority.0;
                add_update("priority", Box::new(priority.0));
                if priority.0 != old_priority {
                    ctx.record_field_change(
                        EventType::PriorityChanged,
                        id,
                        Some(old_priority.to_string()),
                        Some(priority.0.to_string()),
                        None,
                    );
                }
            }

            // Issue type
            if let Some(ref issue_type) = updates.issue_type {
                add_update("issue_type", Box::new(issue_type.as_str().to_string()));
            }

            // Assignee
            if let Some(ref assignee_opt) = updates.assignee {
                let old_assignee = existing.assignee.clone();
                add_update("assignee", Box::new(assignee_opt.clone()));
                if old_assignee != *assignee_opt {
                    ctx.record_field_change(
                        EventType::AssigneeChanged,
                        id,
                        old_assignee,
                        assignee_opt.clone(),
                        None,
                    );
                }
            }

            // Simple Option fields
            if let Some(ref val) = updates.owner {
                add_update("owner", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.estimated_minutes {
                add_update("estimated_minutes", Box::new(*val));
            }
            if let Some(ref val) = updates.external_ref {
                add_update("external_ref", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.close_reason {
                add_update("close_reason", Box::new(val.clone()));
            }
            if let Some(ref val) = updates.closed_by_session {
                add_update("closed_by_session", Box::new(val.clone()));
            }

            // Date fields
            if let Some(ref val) = updates.due_at {
                add_update("due_at", Box::new(val.map(|d| d.to_rfc3339())));
            }
            if let Some(ref val) = updates.defer_until {
                add_update("defer_until", Box::new(val.map(|d| d.to_rfc3339())));
            }
            if let Some(ref val) = updates.closed_at {
                add_update("closed_at", Box::new(val.map(|d| d.to_rfc3339())));
            }

            // No updates? Just return
            if set_clauses.is_empty() {
                return Ok(());
            }

            // Always update updated_at
            set_clauses.push("updated_at = ?".to_string());
            params.push(Box::new(Utc::now().to_rfc3339()));

            // Build and execute SQL
            let sql = format!("UPDATE issues SET {} WHERE id = ?", set_clauses.join(", "));
            params.push(Box::new(id.to_string()));

            let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(AsRef::as_ref).collect();
            tx.execute(&sql, params_refs.as_slice())?;

            ctx.mark_dirty(id);

            Ok(())
        })?;

        // Return updated issue
        self.get_issue(id)?
            .ok_or_else(|| BeadsError::IssueNotFound { id: id.to_string() })
    }

    /// Delete an issue by creating a tombstone.
    ///
    /// # Errors
    ///
    /// Returns an error if the issue doesn't exist or the update fails.
    pub fn delete_issue(&mut self, id: &str, actor: &str, reason: &str) -> Result<Issue> {
        let issue = self
            .get_issue(id)?
            .ok_or_else(|| BeadsError::IssueNotFound { id: id.to_string() })?;

        let original_type = issue.issue_type.as_str().to_string();

        self.mutate("delete_issue", actor, |tx, ctx| {
            tx.execute(
                "UPDATE issues SET
                    status = 'tombstone',
                    deleted_at = ?,
                    deleted_by = ?,
                    delete_reason = ?,
                    original_type = ?,
                    updated_at = ?
                 WHERE id = ?",
                rusqlite::params![
                    Utc::now().to_rfc3339(),
                    actor,
                    reason,
                    original_type,
                    Utc::now().to_rfc3339(),
                    id
                ],
            )?;

            ctx.record_event(
                EventType::Deleted,
                id,
                Some(format!("Deleted issue: {reason}")),
            );
            ctx.mark_dirty(id);
            ctx.invalidate_cache();

            Ok(())
        })?;

        self.get_issue(id)?
            .ok_or_else(|| BeadsError::IssueNotFound { id: id.to_string() })
    }

    /// Get an issue by ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_issue(&self, id: &str) -> Result<Option<Issue>> {
        let sql = r" 
            SELECT id, content_hash, title, description, design, acceptance_criteria, notes,
                   status, priority, issue_type, assignee, owner, estimated_minutes,
                   created_at, created_by, updated_at, closed_at, close_reason, closed_by_session,
                   due_at, defer_until, external_ref, source_system,
                   deleted_at, deleted_by, delete_reason, original_type,
                   compaction_level, compacted_at, compacted_at_commit, original_size,
                   sender, ephemeral, pinned, is_template
            FROM issues WHERE id = ?
        ";

        let mut stmt = self.conn.prepare(sql)?;
        let result = stmt.query_row([id], |row| self.issue_from_row(row));

        match result {
            Ok(issue) => Ok(Some(issue)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// List issues with optional filters.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn list_issues(&self, filters: &ListFilters) -> Result<Vec<Issue>> {
        let mut sql = String::from(
            r"SELECT id, content_hash, title, description, design, acceptance_criteria, notes,
                     status, priority, issue_type, assignee, owner, estimated_minutes,
                     created_at, created_by, updated_at, closed_at, close_reason, closed_by_session,
                     due_at, defer_until, external_ref, source_system,
                     deleted_at, deleted_by, delete_reason, original_type,
                     compaction_level, compacted_at, compacted_at_commit, original_size,
                     sender, ephemeral, pinned, is_template
              FROM issues WHERE 1=1",
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        if let Some(ref statuses) = filters.statuses {
            if !statuses.is_empty() {
                let placeholders: Vec<String> = statuses.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND status IN ({})", placeholders.join(","));
                for s in statuses {
                    params.push(Box::new(s.as_str().to_string()));
                }
            }
        }

        if let Some(ref types) = filters.types {
            if !types.is_empty() {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND issue_type IN ({})", placeholders.join(","));
                for t in types {
                    params.push(Box::new(t.as_str().to_string()));
                }
            }
        }

        if let Some(ref priorities) = filters.priorities {
            if !priorities.is_empty() {
                let placeholders: Vec<String> =
                    priorities.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND priority IN ({})", placeholders.join(","));
                for p in priorities {
                    params.push(Box::new(p.0));
                }
            }
        }

        if let Some(ref assignee) = filters.assignee {
            sql.push_str(" AND assignee = ?");
            params.push(Box::new(assignee.clone()));
        }

        if filters.unassigned {
            sql.push_str(" AND assignee IS NULL");
        }

        if !filters.include_closed {
            sql.push_str(" AND status NOT IN ('closed', 'tombstone')");
        }

        if !filters.include_templates {
            sql.push_str(" AND (is_template = 0 OR is_template IS NULL)");
        }

        if let Some(ref title_contains) = filters.title_contains {
            sql.push_str(" AND title LIKE ?");
            params.push(Box::new(format!("%{title_contains}%")));
        }

        sql.push_str(" ORDER BY priority ASC, created_at DESC");

        if let Some(limit) = filters.limit {
            if limit > 0 {
                let _ = write!(sql, " LIMIT {limit}");
            }
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(AsRef::as_ref).collect();
        let issues = stmt
            .query_map(params_refs.as_slice(), |row| self.issue_from_row(row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    /// Search issues by query with optional filters.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn search_issues(&self, query: &str, filters: &ListFilters) -> Result<Vec<Issue>> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Ok(Vec::new());
        }

        let mut sql = String::from(
            r"SELECT id, content_hash, title, description, design, acceptance_criteria, notes,
                     status, priority, issue_type, assignee, owner, estimated_minutes,
                     created_at, created_by, updated_at, closed_at, close_reason, closed_by_session,
                     due_at, defer_until, external_ref, source_system,
                     deleted_at, deleted_by, delete_reason, original_type,
                     compaction_level, compacted_at, compacted_at_commit, original_size,
                     sender, ephemeral, pinned, is_template
              FROM issues WHERE 1=1",
        );

        let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

        sql.push_str(" AND (title LIKE ? OR description LIKE ? OR id LIKE ?)");
        let pattern = format!("%{trimmed}%");
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern.clone()));
        params.push(Box::new(pattern));

        if let Some(ref statuses) = filters.statuses {
            if !statuses.is_empty() {
                let placeholders: Vec<String> = statuses.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND status IN ({})", placeholders.join(","));
                for s in statuses {
                    params.push(Box::new(s.as_str().to_string()));
                }
            }
        }

        if let Some(ref types) = filters.types {
            if !types.is_empty() {
                let placeholders: Vec<String> = types.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND issue_type IN ({})", placeholders.join(","));
                for t in types {
                    params.push(Box::new(t.as_str().to_string()));
                }
            }
        }

        if let Some(ref priorities) = filters.priorities {
            if !priorities.is_empty() {
                let placeholders: Vec<String> =
                    priorities.iter().map(|_| "?".to_string()).collect();
                let _ = write!(sql, " AND priority IN ({})", placeholders.join(","));
                for p in priorities {
                    params.push(Box::new(p.0));
                }
            }
        }

        if let Some(ref assignee) = filters.assignee {
            sql.push_str(" AND assignee = ?");
            params.push(Box::new(assignee.clone()));
        }

        if filters.unassigned {
            sql.push_str(" AND assignee IS NULL");
        }

        if !filters.include_closed {
            sql.push_str(" AND status NOT IN ('closed', 'tombstone')");
        }

        if !filters.include_templates {
            sql.push_str(" AND (is_template = 0 OR is_template IS NULL)");
        }

        if let Some(ref title_contains) = filters.title_contains {
            sql.push_str(" AND title LIKE ?");
            params.push(Box::new(format!("%{title_contains}%")));
        }

        sql.push_str(" ORDER BY priority ASC, created_at DESC");

        if let Some(limit) = filters.limit {
            if limit > 0 {
                let _ = write!(sql, " LIMIT {limit}");
            }
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let params_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(AsRef::as_ref).collect();
        let issues = stmt
            .query_map(params_refs.as_slice(), |row| self.issue_from_row(row))?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(issues)
    }

    /// Check if an issue ID already exists.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn id_exists(&self, id: &str) -> Result<bool> {
        let count: i64 =
            self.conn
                .query_row("SELECT count(*) FROM issues WHERE id = ?", [id], |row| {
                    row.get(0)
                })?;
        Ok(count > 0)
    }

    /// Find issue IDs that end with the given hash substring.
    pub fn find_ids_by_hash(&self, hash_suffix: &str) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM issues WHERE id LIKE ?")?;
        let pattern = format!("%-%{hash_suffix}%");
        let ids = stmt
            .query_map([pattern], |row| row.get(0))?
            .collect::<std::result::Result<Vec<String>, _>>()?;
        Ok(ids)
    }

    /// Count total issues in the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn count_issues(&self) -> Result<usize> {
        let count: i64 = self
            .conn
            .query_row("SELECT count(*) FROM issues", [], |row| row.get(0))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Get all issue IDs in the database.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_all_ids(&self) -> Result<Vec<String>> {
        let mut stmt = self.conn.prepare("SELECT id FROM issues ORDER BY id")?;
        let ids = stmt
            .query_map([], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Add a dependency between issues.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn add_dependency(
        &mut self,
        issue_id: &str,
        depends_on_id: &str,
        dep_type: &str,
        actor: &str,
    ) -> Result<bool> {
        self.mutate("add_dependency", actor, |tx, ctx| {
            let exists: i64 = tx.query_row(
                "SELECT count(*) FROM dependencies WHERE issue_id = ? AND depends_on_id = ?",
                rusqlite::params![issue_id, depends_on_id],
                |row| row.get(0),
            )?;

            if exists > 0 {
                return Ok(false);
            }

            tx.execute(
                "INSERT INTO dependencies (issue_id, depends_on_id, type, created_at, created_by)
                 VALUES (?, ?, ?, ?, ?)",
                rusqlite::params![
                    issue_id,
                    depends_on_id,
                    dep_type,
                    Utc::now().to_rfc3339(),
                    actor
                ],
            )?;

            ctx.record_event(
                EventType::DependencyAdded,
                issue_id,
                Some(format!("Added dependency on {depends_on_id} ({dep_type})")),
            );
            ctx.mark_dirty(issue_id);
            ctx.invalidate_cache();

            Ok(true)
        })
    }

    /// Remove a dependency link.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn remove_dependency(
        &mut self,
        issue_id: &str,
        depends_on_id: &str,
        actor: &str,
    ) -> Result<bool> {
        self.mutate("remove_dependency", actor, |tx, ctx| {
            let rows = tx.execute(
                "DELETE FROM dependencies WHERE issue_id = ? AND depends_on_id = ?",
                rusqlite::params![issue_id, depends_on_id],
            )?;

            if rows > 0 {
                ctx.record_event(
                    EventType::DependencyRemoved,
                    issue_id,
                    Some(format!("Removed dependency on {depends_on_id}")),
                );
                ctx.mark_dirty(issue_id);
                ctx.invalidate_cache();
            }

            Ok(rows > 0)
        })
    }

    /// Remove all dependencies for an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn remove_all_dependencies(&mut self, issue_id: &str, actor: &str) -> Result<usize> {
        self.mutate("remove_all_dependencies", actor, |tx, ctx| {
            let mut stmt = tx.prepare(
                "SELECT DISTINCT issue_id FROM dependencies WHERE depends_on_id = ?
                 UNION
                 SELECT DISTINCT depends_on_id FROM dependencies WHERE issue_id = ?",
            )?;
            let affected: Vec<String> = stmt
                .query_map(rusqlite::params![issue_id, issue_id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;

            let outgoing = tx.execute("DELETE FROM dependencies WHERE issue_id = ?", [issue_id])?;
            let incoming = tx.execute(
                "DELETE FROM dependencies WHERE depends_on_id = ?",
                [issue_id],
            )?;
            let total = outgoing + incoming;

            if total > 0 {
                ctx.record_event(
                    EventType::DependencyRemoved,
                    issue_id,
                    Some(format!("Removed {total} dependency links")),
                );
                ctx.mark_dirty(issue_id);
                for affected_id in affected {
                    ctx.mark_dirty(&affected_id);
                }
                ctx.invalidate_cache();
            }

            Ok(total)
        })
    }

    /// Remove parent-child dependency for an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn remove_parent(&mut self, issue_id: &str, actor: &str) -> Result<bool> {
        self.mutate("remove_parent", actor, |tx, ctx| {
            let rows = tx.execute(
                "DELETE FROM dependencies WHERE issue_id = ? AND type = 'parent-child'",
                rusqlite::params![issue_id],
            )?;

            if rows > 0 {
                ctx.record_event(
                    EventType::DependencyRemoved,
                    issue_id,
                    Some("Removed parent".to_string()),
                );
                ctx.mark_dirty(issue_id);
                ctx.invalidate_cache();
            }

            Ok(rows > 0)
        })
    }

    /// Add a label to an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn add_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<bool> {
        self.mutate("add_label", actor, |tx, ctx| {
            let exists: i64 = tx.query_row(
                "SELECT count(*) FROM labels WHERE issue_id = ? AND label = ?",
                rusqlite::params![issue_id, label],
                |row| row.get(0),
            )?;

            if exists > 0 {
                return Ok(false);
            }

            tx.execute(
                "INSERT INTO labels (issue_id, label) VALUES (?, ?)",
                rusqlite::params![issue_id, label],
            )?;

            ctx.record_event(
                EventType::LabelAdded,
                issue_id,
                Some(format!("Added label {label}")),
            );
            ctx.mark_dirty(issue_id);

            Ok(true)
        })
    }

    /// Remove a label from an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn remove_label(&mut self, issue_id: &str, label: &str, actor: &str) -> Result<bool> {
        self.mutate("remove_label", actor, |tx, ctx| {
            let rows = tx.execute(
                "DELETE FROM labels WHERE issue_id = ? AND label = ?",
                rusqlite::params![issue_id, label],
            )?;

            if rows > 0 {
                ctx.record_event(
                    EventType::LabelRemoved,
                    issue_id,
                    Some(format!("Removed label {label}")),
                );
                ctx.mark_dirty(issue_id);
            }

            Ok(rows > 0)
        })
    }

    /// Remove all labels from an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn remove_all_labels(&mut self, issue_id: &str, actor: &str) -> Result<usize> {
        self.mutate("remove_all_labels", actor, |tx, ctx| {
            let rows = tx.execute(
                "DELETE FROM labels WHERE issue_id = ?",
                rusqlite::params![issue_id],
            )?;

            if rows > 0 {
                ctx.record_event(
                    EventType::LabelRemoved,
                    issue_id,
                    Some(format!("Removed {rows} labels")),
                );
                ctx.mark_dirty(issue_id);
            }

            Ok(rows)
        })
    }

    /// Set all labels for an issue (replace existing).
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn set_labels(&mut self, issue_id: &str, labels: &[String], actor: &str) -> Result<()> {
        self.mutate("set_labels", actor, |tx, ctx| {
            let mut stmt = tx.prepare("SELECT label FROM labels WHERE issue_id = ?")?;
            let old_labels: Vec<String> = stmt
                .query_map([issue_id], |row| row.get(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            drop(stmt);

            tx.execute("DELETE FROM labels WHERE issue_id = ?", [issue_id])?;

            for label in labels {
                tx.execute(
                    "INSERT INTO labels (issue_id, label) VALUES (?, ?)",
                    rusqlite::params![issue_id, label],
                )?;
            }

            // Record changes
            let removed: Vec<_> = old_labels.iter().filter(|l| !labels.contains(l)).collect();
            let added: Vec<_> = labels.iter().filter(|l| !old_labels.contains(l)).collect();

            if !removed.is_empty() || !added.is_empty() {
                let mut details = Vec::new();
                if !removed.is_empty() {
                    details.push(format!(
                        "removed: {}",
                        removed
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                if !added.is_empty() {
                    details.push(format!(
                        "added: {}",
                        added
                            .iter()
                            .map(|s| s.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                ctx.record_event(
                    EventType::Updated,
                    issue_id,
                    Some(format!("Labels {}", details.join("; "))),
                );
                ctx.mark_dirty(issue_id);
            }

            Ok(())
        })
    }

    /// Get labels for an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_labels(&self, issue_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT label FROM labels WHERE issue_id = ? ORDER BY label")?;
        let labels = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(labels)
    }

    /// Get comments for an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_comments(&self, issue_id: &str) -> Result<Vec<Comment>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, issue_id, author, text, created_at
             FROM comments
             WHERE issue_id = ?
             ORDER BY created_at ASC",
        )?;

        let comments = stmt
            .query_map([issue_id], |row| {
                Ok(Comment {
                    id: row.get(0)?,
                    issue_id: row.get(1)?,
                    author: row.get(2)?,
                    body: row.get(3)?,
                    created_at: parse_datetime(&row.get::<_, String>(4)?),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(comments)
    }

    /// Add a comment to an issue.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn add_comment(&mut self, issue_id: &str, author: &str, text: &str) -> Result<Comment> {
        self.mutate("add_comment", author, |tx, ctx| {
            let comment_id = insert_comment_row(tx, issue_id, author, text)?;

            tx.execute(
                "UPDATE issues SET updated_at = CURRENT_TIMESTAMP WHERE id = ?",
                rusqlite::params![issue_id],
            )?;

            ctx.record_event(EventType::Commented, issue_id, Some(text.to_string()));
            ctx.mark_dirty(issue_id);

            fetch_comment(tx, comment_id)
        })
    }

    /// Get dependencies with metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_dependencies_with_metadata(
        &self,
        issue_id: &str,
    ) -> Result<Vec<IssueWithDependencyMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.depends_on_id, i.title, i.status, i.priority, d.type
             FROM dependencies d
             LEFT JOIN issues i ON d.depends_on_id = i.id
             WHERE d.issue_id = ?
             ORDER BY i.priority ASC, i.created_at DESC",
        )?;

        let deps = stmt
            .query_map([issue_id], |row| {
                Ok(IssueWithDependencyMetadata {
                    id: row.get(0)?,
                    title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    status: parse_status(row.get::<_, Option<String>>(2)?.as_deref()),
                    priority: Priority(row.get::<_, Option<i32>>(3)?.unwrap_or(2)),
                    dep_type: row
                        .get::<_, Option<String>>(4)?
                        .unwrap_or_else(|| "blocks".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(deps)
    }

    /// Get dependents with metadata.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_dependents_with_metadata(
        &self,
        issue_id: &str,
    ) -> Result<Vec<IssueWithDependencyMetadata>> {
        let mut stmt = self.conn.prepare(
            "SELECT d.issue_id, i.title, i.status, i.priority, d.type
             FROM dependencies d
             LEFT JOIN issues i ON d.issue_id = i.id
             WHERE d.depends_on_id = ?
             ORDER BY i.priority ASC, i.created_at DESC",
        )?;

        let deps = stmt
            .query_map([issue_id], |row| {
                Ok(IssueWithDependencyMetadata {
                    id: row.get(0)?,
                    title: row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                    status: parse_status(row.get::<_, Option<String>>(2)?.as_deref()),
                    priority: Priority(row.get::<_, Option<i32>>(3)?.unwrap_or(2)),
                    dep_type: row
                        .get::<_, Option<String>>(4)?
                        .unwrap_or_else(|| "blocks".to_string()),
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        Ok(deps)
    }

    /// Get parent issue ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_parent_id(&self, issue_id: &str) -> Result<Option<String>> {
        let result = self.conn.query_row(
            "SELECT depends_on_id FROM dependencies WHERE issue_id = ? AND type = 'parent-child'",
            [issue_id],
            |row| row.get(0),
        );

        match result {
            Ok(parent_id) => Ok(Some(parent_id)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get IDs of issues that depend on this one.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_dependents(&self, issue_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT issue_id FROM dependencies WHERE depends_on_id = ?")?;
        let ids = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Get IDs of issues that this one depends on.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_dependencies(&self, issue_id: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT depends_on_id FROM dependencies WHERE issue_id = ?")?;
        let ids = stmt
            .query_map([issue_id], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(ids)
    }

    /// Count how many dependencies an issue has.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn count_dependencies(&self, issue_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM dependencies WHERE issue_id = ?",
            [issue_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Count how many issues depend on this one.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn count_dependents(&self, issue_id: &str) -> Result<usize> {
        let count: i64 = self.conn.query_row(
            "SELECT count(*) FROM dependencies WHERE depends_on_id = ?",
            [issue_id],
            |row| row.get(0),
        )?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Fetch a config value.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_config(&self, key: &str) -> Result<Option<String>> {
        let value = self
            .conn
            .query_row("SELECT value FROM config WHERE key = ?", [key], |row| {
                row.get(0)
            })
            .optional()?;
        Ok(value)
    }

    /// Fetch all config values from the config table.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_all_config(&self) -> Result<HashMap<String, String>> {
        let mut stmt = self.conn.prepare("SELECT key, value FROM config")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        let mut map = HashMap::new();
        for row in rows {
            let (key, value) = row?;
            map.insert(key, value);
        }
        Ok(map)
    }

    /// Set a config value.
    ///
    /// # Errors
    ///
    /// Returns an error if the database update fails.
    pub fn set_config(&mut self, key: &str, value: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO config (key, value) VALUES (?, ?)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            rusqlite::params![key, value],
        )?;
        Ok(())
    }

    /// Get full issue details.
    ///
    /// # Errors
    ///
    /// Returns an error if the database query fails.
    pub fn get_issue_details(
        &self,
        id: &str,
        include_comments: bool,
        include_events: bool,
        event_limit: usize,
    ) -> Result<Option<IssueDetails>> {
        let Some(issue) = self.get_issue(id)? else {
            return Ok(None);
        };

        let labels = self.get_labels(id)?;
        let dependencies = self.get_dependencies_with_metadata(id)?;
        let dependents = self.get_dependents_with_metadata(id)?;
        let comments = if include_comments {
            self.get_comments(id)?
        } else {
            vec![]
        };
        let events = if include_events {
            get_events(&self.conn, id, event_limit)?
        } else {
            vec![]
        };
        let parent = self.get_parent_id(id)?;

        Ok(Some(IssueDetails {
            issue,
            labels,
            dependencies,
            dependents,
            comments,
            events,
            parent,
        }))
    }

    #[allow(clippy::unused_self)]
    fn issue_from_row(&self, row: &rusqlite::Row) -> rusqlite::Result<Issue> {
        Ok(Issue {
            id: row.get(0)?,
            content_hash: row.get(1)?,
            title: row.get(2)?,
            description: row.get(3)?,
            design: row.get(4)?,
            acceptance_criteria: row.get(5)?,
            notes: row.get(6)?,
            status: parse_status(row.get::<_, Option<String>>(7)?.as_deref()),
            priority: Priority(row.get::<_, Option<i32>>(8)?.unwrap_or(2)),
            issue_type: parse_issue_type(row.get::<_, Option<String>>(9)?.as_deref()),
            assignee: row.get(10)?,
            owner: row.get(11)?,
            estimated_minutes: row.get(12)?,
            created_at: parse_datetime(&row.get::<_, String>(13)?),
            created_by: row.get(14)?,
            updated_at: parse_datetime(&row.get::<_, String>(15)?),
            closed_at: row
                .get::<_, Option<String>>(16)?
                .as_deref()
                .map(parse_datetime),
            close_reason: row.get(17)?,
            closed_by_session: row.get(18)?,
            due_at: row
                .get::<_, Option<String>>(19)?
                .as_deref()
                .map(parse_datetime),
            defer_until: row
                .get::<_, Option<String>>(20)?
                .as_deref()
                .map(parse_datetime),
            external_ref: row.get(21)?,
            source_system: row.get(22)?,
            deleted_at: row
                .get::<_, Option<String>>(23)?
                .as_deref()
                .map(parse_datetime),
            deleted_by: row.get(24)?,
            delete_reason: row.get(25)?,
            original_type: row.get(26)?,
            compaction_level: row.get(27)?,
            compacted_at: row
                .get::<_, Option<String>>(28)?
                .as_deref()
                .map(parse_datetime),
            compacted_at_commit: row.get(29)?,
            original_size: row.get(30)?,
            sender: row.get(31)?,
            ephemeral: row.get::<_, Option<i32>>(32)?.unwrap_or(0) != 0,
            pinned: row.get::<_, Option<i32>>(33)?.unwrap_or(0) != 0,
            is_template: row.get::<_, Option<i32>>(34)?.unwrap_or(0) != 0,
            labels: vec![],       // Loaded separately if needed
            dependencies: vec![], // Loaded separately if needed
            comments: vec![],     // Loaded separately if needed
        })
    }
}

/// Filter options for listing issues.
#[derive(Debug, Clone, Default)]
pub struct ListFilters {
    pub statuses: Option<Vec<Status>>,
    pub types: Option<Vec<IssueType>>,
    pub priorities: Option<Vec<Priority>>,
    pub assignee: Option<String>,
    pub unassigned: bool,
    pub include_closed: bool,
    pub include_templates: bool,
    pub title_contains: Option<String>,
    pub limit: Option<usize>,
}

/// Fields to update on an issue.
#[derive(Debug, Clone, Default)]
pub struct IssueUpdate {
    pub title: Option<String>,
    pub description: Option<Option<String>>,
    pub design: Option<Option<String>>,
    pub acceptance_criteria: Option<Option<String>>,
    pub notes: Option<Option<String>>,
    pub status: Option<Status>,
    pub priority: Option<Priority>,
    pub issue_type: Option<IssueType>,
    pub assignee: Option<Option<String>>,
    pub owner: Option<Option<String>>,
    pub estimated_minutes: Option<Option<i32>>,
    pub due_at: Option<Option<DateTime<Utc>>>,
    pub defer_until: Option<Option<DateTime<Utc>>>,
    pub external_ref: Option<Option<String>>,
    pub closed_at: Option<Option<DateTime<Utc>>>,
    pub close_reason: Option<Option<String>>,
    pub closed_by_session: Option<Option<String>>,
}

impl IssueUpdate {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.description.is_none()
            && self.design.is_none()
            && self.acceptance_criteria.is_none()
            && self.notes.is_none()
            && self.status.is_none()
            && self.priority.is_none()
            && self.issue_type.is_none()
            && self.assignee.is_none()
            && self.owner.is_none()
            && self.estimated_minutes.is_none()
            && self.due_at.is_none()
            && self.defer_until.is_none()
            && self.external_ref.is_none()
            && self.closed_at.is_none()
            && self.close_reason.is_none()
            && self.closed_by_session.is_none()
    }
}

fn parse_status(s: Option<&str>) -> Status {
    s.and_then(|s| s.parse().ok()).unwrap_or_default()
}

fn parse_issue_type(s: Option<&str>) -> IssueType {
    s.and_then(|s| s.parse().ok()).unwrap_or_default()
}

fn parse_datetime(s: &str) -> DateTime<Utc> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.with_timezone(&Utc);
    }

    if let Ok(naive) = NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Utc.from_utc_datetime(&naive);
    }

    Utc::now()
}

fn insert_comment_row(
    tx: &Transaction<'_>,
    issue_id: &str,
    author: &str,
    text: &str,
) -> Result<i64> {
    tx.execute(
        "INSERT INTO comments (issue_id, author, text, created_at)
         VALUES (?, ?, ?, CURRENT_TIMESTAMP)",
        rusqlite::params![issue_id, author, text],
    )?;
    Ok(tx.last_insert_rowid())
}

fn fetch_comment(tx: &Transaction<'_>, comment_id: i64) -> Result<Comment> {
    tx.query_row(
        "SELECT id, issue_id, author, text, created_at FROM comments WHERE id = ?",
        rusqlite::params![comment_id],
        |row| {
            Ok(Comment {
                id: row.get(0)?,
                issue_id: row.get(1)?,
                author: row.get(2)?,
                body: row.get(3)?,
                created_at: parse_datetime(&row.get::<_, String>(4)?),
            })
        },
    )
    .map_err(BeadsError::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Issue, IssueType, Priority, Status};

    #[test]
    fn test_open_memory() {
        let storage = SqliteStorage::open_memory();
        assert!(storage.is_ok());
    }

    #[test]
    fn test_create_issue() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue = Issue {
            id: "bd-1".to_string(),
            title: "Test Issue".to_string(),
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            content_hash: None,
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        };

        storage.create_issue(&issue, "tester").unwrap();

        // Verify it exists (raw query since get_issue not impl yet)
        let count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM issues WHERE id = ?",
                ["bd-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Verify event
        let event_count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM events WHERE issue_id = ?",
                ["bd-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(event_count, 1);

        // Verify dirty
        let dirty_count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM dirty_issues WHERE issue_id = ?",
                ["bd-1"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dirty_count, 1);
    }

    #[test]
    fn test_transaction_rollback_on_error() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        // Try to create an issue that will fail validation (title too long)
        let result: crate::error::Result<()> = storage.mutate("test_fail", "tester", |tx, ctx| {
            // Insert successfully first
            tx.execute(
                "INSERT INTO issues (id, title, status, priority, issue_type, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
                rusqlite::params![
                    "bd-rollback",
                    "Valid title",
                    "open",
                    2,
                    "task",
                    Utc::now().to_rfc3339(),
                    Utc::now().to_rfc3339(),
                ],
            )?;
            ctx.mark_dirty("bd-rollback");

            // Now force an error
            Err(crate::error::BeadsError::IssueNotFound {
                id: "forced".into(),
            })
        });

        assert!(result.is_err());

        // Issue should NOT exist due to rollback
        let count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM issues WHERE id = ?",
                ["bd-rollback"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0, "Issue should not exist after rollback");

        // Dirty marker should NOT exist due to rollback
        let dirty_count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM dirty_issues WHERE issue_id = ?",
                ["bd-rollback"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            dirty_count, 0,
            "Dirty marker should not exist after rollback"
        );
    }

    #[test]
    fn test_dirty_issues_accumulate() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        // Create first issue
        let issue1 = Issue {
            id: "bd-dirty1".to_string(),
            title: "First".to_string(),
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            content_hash: None,
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        };
        storage.create_issue(&issue1, "tester").unwrap();

        // Create second issue
        let issue2 = Issue {
            id: "bd-dirty2".to_string(),
            title: "Second".to_string(),
            ..issue1.clone()
        };
        storage.create_issue(&issue2, "tester").unwrap();

        // Both should be dirty
        let dirty_count: i64 = storage
            .conn
            .query_row("SELECT count(*) FROM dirty_issues", [], |row| row.get(0))
            .unwrap();
        assert_eq!(dirty_count, 2, "Both issues should be marked dirty");

        // Clear dirty for one
        storage
            .conn
            .execute("DELETE FROM dirty_issues WHERE issue_id = ?", ["bd-dirty1"])
            .unwrap();

        // One should remain dirty
        let dirty_count: i64 = storage
            .conn
            .query_row("SELECT count(*) FROM dirty_issues", [], |row| row.get(0))
            .unwrap();
        assert_eq!(dirty_count, 1, "One issue should remain dirty");
    }

    #[test]
    fn test_add_comment_round_trip() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        let issue = Issue {
            id: "bd-comment".to_string(),
            title: "Comment issue".to_string(),
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            content_hash: None,
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        };
        storage.create_issue(&issue, "tester").unwrap();

        let comment = storage
            .add_comment("bd-comment", "alice", "Hello there")
            .unwrap();
        assert_eq!(comment.issue_id, "bd-comment");
        assert_eq!(comment.author, "alice");
        assert_eq!(comment.body, "Hello there");
        assert!(comment.id > 0);

        let comments = storage.get_comments("bd-comment").unwrap();
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0], comment);
    }

    #[test]
    fn test_add_comment_marks_dirty() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        let issue = Issue {
            id: "bd-comment-dirty".to_string(),
            title: "Comment issue".to_string(),
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            content_hash: None,
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        };
        storage.create_issue(&issue, "tester").unwrap();

        storage
            .add_comment("bd-comment-dirty", "alice", "Dirty comment")
            .unwrap();

        let dirty_count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM dirty_issues WHERE issue_id = ?",
                ["bd-comment-dirty"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(dirty_count, 1);
    }

    #[test]
    fn test_events_have_timestamps() {
        let mut storage = SqliteStorage::open_memory().unwrap();
        let issue = Issue {
            id: "bd-events".to_string(),
            title: "Event Test".to_string(),
            status: Status::Open,
            priority: Priority::MEDIUM,
            issue_type: IssueType::Task,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            content_hash: None,
            description: None,
            design: None,
            acceptance_criteria: None,
            notes: None,
            assignee: None,
            owner: None,
            estimated_minutes: None,
            created_by: None,
            closed_at: None,
            close_reason: None,
            closed_by_session: None,
            due_at: None,
            defer_until: None,
            external_ref: None,
            source_system: None,
            deleted_at: None,
            deleted_by: None,
            delete_reason: None,
            original_type: None,
            compaction_level: None,
            compacted_at: None,
            compacted_at_commit: None,
            original_size: None,
            sender: None,
            ephemeral: false,
            pinned: false,
            is_template: false,
            labels: vec![],
            dependencies: vec![],
            comments: vec![],
        };
        storage.create_issue(&issue, "tester").unwrap();

        // Verify event has timestamp
        let created_at: String = storage
            .conn
            .query_row(
                "SELECT created_at FROM events WHERE issue_id = ?",
                ["bd-events"],
                |row| row.get(0),
            )
            .unwrap();

        // Should be a valid RFC3339 timestamp
        assert!(
            chrono::DateTime::parse_from_rfc3339(&created_at).is_ok(),
            "Event timestamp should be valid RFC3339"
        );
    }

    #[test]
    fn test_blocked_cache_invalidation() {
        let mut storage = SqliteStorage::open_memory().unwrap();

        // Manually insert some cache data
        storage
            .conn
            .execute(
                "INSERT INTO blocked_issues_cache (issue_id, blocked_by_json) VALUES (?, ?)",
                ["bd-cached", r#"["bd-blocker"]"#],
            )
            .unwrap();

        // Verify cache has data
        let count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM blocked_issues_cache WHERE issue_id = ?",
                ["bd-cached"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);

        // Now add a dependency
        storage
            .add_dependency("bd-cached", "bd-blocker", "blocks", "tester")
            .unwrap();

        // Cache should be invalidated
        let count: i64 = storage
            .conn
            .query_row(
                "SELECT count(*) FROM blocked_issues_cache WHERE issue_id = ?",
                ["bd-cached"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 0);
    }
}
