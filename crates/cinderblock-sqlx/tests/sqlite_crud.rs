// # SQLite Data Layer Integration Tests
//
// End-to-end tests that verify the full pipeline:
//   resource! macro → cinderblock_sqlx extension codegen → SqliteDataLayer CRUD
//
// Each test creates a fresh in-memory SQLite database, applies the schema,
// registers the data layer on a Context, and exercises the cinderblock_core CRUD
// functions against a real database.

use std::sync::Arc;

use assert2::{assert, check};
use cinderblock_core::{
    Context, resource,
    serde::{Deserialize, Serialize},
};
use cinderblock_sqlx::sqlite::SqliteDataLayer;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// # Test Resource Definition
// ---------------------------------------------------------------------------

// A custom enum stored as TEXT in SQLite. sqlx's derive macro handles the
// string conversion — variant names are used as-is (e.g., "Low", "Medium").
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, sqlx::Type)]
enum Priority {
    #[default]
    Low,
    Medium,
    High,
}

resource! {
    name = Test.Task;
    data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;

    attributes {
        task_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        title String;
        priority Priority;
        done bool;
    }

    actions {
        read all;

        read important_tasks {
            filter { priority == Priority::High };
        }

        read open_tasks {
            filter { done == false };
        }

        read by_priority {
            argument { priority: Priority };
            filter { priority == arg(priority) };
        }

        read by_priority_and_status {
            argument { priority: Priority, done: Option<bool> };
            filter { priority == arg(priority) };
            filter { done == arg(done) };
        }

        read paged_all {
            paged;
        }

        read paged_by_priority {
            argument { priority: Priority };
            filter { priority == arg(priority) };

            paged {
                default_per_page 3;
                max_per_page 5;
            };
        }

        create add;

        update complete {
            accept [];
            change_ref |task| {
                task.done = true;
            };
        };

        destroy remove;
    }

    extensions {
        cinderblock_sqlx {
            table = "tasks";
        };
    }
}

// ---------------------------------------------------------------------------
// # Test Setup
// ---------------------------------------------------------------------------

/// Create a fresh in-memory SQLite database with the `tasks` table,
/// register the data layer on a new Context, and return both.
///
/// Each call produces an isolated database — tests don't interfere with
/// each other even when run in parallel.
async fn setup() -> (Arc<Context>, SqliteDataLayer) {
    let dl = SqliteDataLayer::new("sqlite::memory:")
        .await
        .expect("connect to in-memory SQLite");

    // Create the table schema matching our resource attributes.
    // Uuid → TEXT, String → TEXT, Priority (sqlx::Type enum) → TEXT, bool → BOOLEAN.
    sqlx::query(
        "CREATE TABLE tasks (
            task_id TEXT NOT NULL PRIMARY KEY,
            title TEXT NOT NULL,
            priority TEXT NOT NULL,
            done BOOLEAN NOT NULL
        )",
    )
    .execute(dl.pool())
    .await
    .expect("create tasks table");

    let mut ctx = Context::new();
    ctx.register_data_layer(dl.clone());

    (Arc::new(ctx), dl)
}

// ---------------------------------------------------------------------------
// # Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn create_and_read_back_via_list() {
    let (ctx, _dl) = setup().await;

    let created = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Write integration tests".to_string(),
            priority: Priority::High,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task");

    let tasks = cinderblock_core::read::<Task, All>(&ctx, &())
        .await
        .expect("list tasks");

    check!(tasks.len() == 1);
    check!(tasks[0].task_id == created.task_id);
    check!(tasks[0].title == "Write integration tests");
    check!(tasks[0].priority == Priority::High);
    check!(tasks[0].done == false);
}

#[tokio::test]
async fn read_by_primary_key() {
    let (ctx, dl) = setup().await;

    let created = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Read test".to_string(),
            priority: Priority::Low,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task");

    // Read directly via the data layer trait (through the context).
    let fetched = cinderblock_core::data_layer::DataLayer::<Task>::read(&dl, &created.task_id)
        .await
        .expect("read task by PK");

    check!(fetched.task_id == created.task_id);
    check!(fetched.title == "Read test");
    check!(fetched.priority == Priority::Low);
    check!(fetched.done == false);
}

#[tokio::test]
async fn update_modifies_row() {
    let (ctx, _dl) = setup().await;

    let created = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Incomplete task".to_string(),
            priority: Priority::Medium,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task");

    check!(created.done == false);

    // The `complete` action sets `done = true` via its change_ref closure.
    let updated =
        cinderblock_core::update::<Task, Complete>(&created.task_id, CompleteInput {}, &ctx)
            .await
            .expect("update task");

    check!(updated.done == true);
    check!(updated.task_id == created.task_id);
    check!(updated.title == "Incomplete task");

    // Verify the change persisted in the database.
    let tasks = cinderblock_core::read::<Task, All>(&ctx, &())
        .await
        .expect("list tasks");

    check!(tasks.len() == 1);
    check!(tasks[0].done == true);
}

#[tokio::test]
async fn list_returns_all_resources() {
    let (ctx, _dl) = setup().await;

    for i in 0..3 {
        cinderblock_core::create::<Task, Add>(
            AddInput {
                title: format!("Task {i}"),
                priority: Priority::Low,
                done: false,
            },
            &ctx,
        )
        .await
        .expect("create task");
    }

    let tasks = cinderblock_core::read::<Task, All>(&ctx, &())
        .await
        .expect("list tasks");

    check!(tasks.len() == 3);

    // Verify all titles are present (order is not guaranteed by SQL without
    // ORDER BY, but SQLite typically returns insertion order).
    let mut titles: Vec<String> = tasks.iter().map(|t| t.title.clone()).collect();
    titles.sort();
    check!(titles == vec!["Task 0", "Task 1", "Task 2"]);
}

#[tokio::test]
async fn destroy_deletes_and_returns_resource() {
    let (ctx, _dl) = setup().await;

    let created = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Doomed task".to_string(),
            priority: Priority::High,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task");

    let destroyed = cinderblock_core::destroy::<Task, Remove>(&created.task_id, &ctx)
        .await
        .expect("destroy task");

    check!(destroyed.task_id == created.task_id);
    check!(destroyed.title == "Doomed task");

    // Verify the table is now empty.
    let tasks = cinderblock_core::read::<Task, All>(&ctx, &())
        .await
        .expect("list tasks");

    check!(tasks.is_empty());
}

#[tokio::test]
async fn destroy_nonexistent_returns_error() {
    let (ctx, _dl) = setup().await;

    let fake_id = Uuid::new_v4();
    let result = cinderblock_core::destroy::<Task, Remove>(&fake_id, &ctx).await;

    assert!(let Err(_) = result);
}

#[tokio::test]
async fn create_multiple_then_destroy_one() {
    let (ctx, _dl) = setup().await;

    let task_a = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Keep me".to_string(),
            priority: Priority::Low,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task A");

    let task_b = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Delete me".to_string(),
            priority: Priority::High,
            done: true,
        },
        &ctx,
    )
    .await
    .expect("create task B");

    cinderblock_core::destroy::<Task, Remove>(&task_b.task_id, &ctx)
        .await
        .expect("destroy task B");

    let remaining = cinderblock_core::read::<Task, All>(&ctx, &())
        .await
        .expect("list tasks");

    check!(remaining.len() == 1);
    check!(remaining[0].task_id == task_a.task_id);
    check!(remaining[0].title == "Keep me");
}

#[tokio::test]
async fn read_actions_with_filter() {
    let (ctx, _dl) = setup().await;

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Not so important".to_string(),
            priority: Priority::Low,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task A");

    let expected = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Very import".to_string(),
            priority: Priority::High,
            done: true,
        },
        &ctx,
    )
    .await
    .expect("create task B");

    let open_tasks = cinderblock_core::read::<Task, ImportantTasks>(&ctx, &())
        .await
        .expect("destroy task B");

    check!(open_tasks.len() == 1);
    check!(open_tasks[0].task_id == expected.task_id);
    check!(open_tasks[0].title == expected.title);
}

#[tokio::test]
async fn read_actions_with_filter_2() {
    let (ctx, _dl) = setup().await;

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Closed".to_string(),
            priority: Priority::Low,
            done: true,
        },
        &ctx,
    )
    .await
    .expect("create task A");

    let expected = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Open".to_string(),
            priority: Priority::Medium,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create task B");

    let open_tasks = cinderblock_core::read::<Task, OpenTasks>(&ctx, &())
        .await
        .expect("destroy task B");

    check!(open_tasks.len() == 1);
    check!(open_tasks[0].task_id == expected.task_id);
    check!(open_tasks[0].title == expected.title);
}

// ---------------------------------------------------------------------------
// # Runtime argument tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn read_action_with_required_argument() {
    let (ctx, _dl) = setup().await;

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "Low priority".to_string(),
            priority: Priority::Low,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create low-priority task");

    let expected = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "High priority".to_string(),
            priority: Priority::High,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create high-priority task");

    // Use the `by_priority` read action with a required argument
    let results = cinderblock_core::read::<Task, ByPriority>(
        &ctx,
        &ByPriorityArguments {
            priority: Priority::High,
        },
    )
    .await
    .expect("read by priority");

    check!(results.len() == 1);
    check!(results[0].task_id == expected.task_id);
    check!(results[0].title == "High priority");
}

#[tokio::test]
async fn read_action_with_optional_argument_some() {
    let (ctx, _dl) = setup().await;

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "High but done".to_string(),
            priority: Priority::High,
            done: true,
        },
        &ctx,
    )
    .await
    .expect("create done high-priority task");

    let expected = cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "High and open".to_string(),
            priority: Priority::High,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create open high-priority task");

    // Both arguments provided — filters on priority AND done
    let results = cinderblock_core::read::<Task, ByPriorityAndStatus>(
        &ctx,
        &ByPriorityAndStatusArguments {
            priority: Priority::High,
            done: Some(false),
        },
    )
    .await
    .expect("read by priority and status");

    check!(results.len() == 1);
    check!(results[0].task_id == expected.task_id);
    check!(results[0].title == "High and open");
}

#[tokio::test]
async fn read_action_with_optional_argument_none() {
    let (ctx, _dl) = setup().await;

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "High done".to_string(),
            priority: Priority::High,
            done: true,
        },
        &ctx,
    )
    .await
    .expect("create done high-priority task");

    cinderblock_core::create::<Task, Add>(
        AddInput {
            title: "High open".to_string(),
            priority: Priority::High,
            done: false,
        },
        &ctx,
    )
    .await
    .expect("create open high-priority task");

    // Optional `done` argument is None — only filters on priority
    let results = cinderblock_core::read::<Task, ByPriorityAndStatus>(
        &ctx,
        &ByPriorityAndStatusArguments {
            priority: Priority::High,
            done: None,
        },
    )
    .await
    .expect("read by priority only");

    check!(results.len() == 2);
}

// ---------------------------------------------------------------------------
// # Database-generated value tests
// ---------------------------------------------------------------------------
//
// This second resource exercises `generated true` on the primary key
// (autoincrement integer) and on a non-PK column (a `created_at` text
// field with a server-side DEFAULT). These columns should be omitted from
// INSERT and UPDATE statements, with their values coming from the database.

resource! {
    name = Test.Note;
    data_layer = cinderblock_sqlx::sqlite::SqliteDataLayer;

    attributes {
        note_id i64 {
            primary_key true;
            generated true;
            writable false;
        }
        body String;
        created_at String {
            generated true;
            writable false;
        }
    }

    actions {
        read all_notes;
        create add_note;
        update edit_note {
            accept [body];
        };
        destroy remove_note;
    }

    extensions {
        cinderblock_sqlx {
            table = "notes";
        };
    }
}

// ---------------------------------------------------------------------------
// # Paged Read Tests
// ---------------------------------------------------------------------------

/// Helper: insert `n` tasks with sequential titles and the given priority.
async fn seed_tasks(ctx: &Context, n: usize, priority: Priority) -> Vec<Task> {
    let mut tasks = Vec::with_capacity(n);
    for i in 0..n {
        let task = cinderblock_core::create::<Task, Add>(
            AddInput {
                title: format!("Task {i}"),
                priority: priority.clone(),
                done: false,
            },
            ctx,
        )
        .await
        .expect("seed task");
        tasks.push(task);
    }
    tasks
}

#[tokio::test]
async fn paged_read_returns_paginated_result() {
    let (ctx, _dl) = setup().await;
    seed_tasks(&ctx, 5, Priority::Low).await;

    let result = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: None,
            per_page: None,
        },
    )
    .await
    .expect("paged read all");

    // Default page is 1, default per_page is DEFAULT_PER_PAGE (100), so all 5
    // tasks fit on the first page.
    check!(result.data.len() == 5);
    check!(result.meta.page == 1);
    check!(result.meta.per_page == cinderblock_core::DEFAULT_PER_PAGE);
    check!(result.meta.total == 5);
    check!(result.meta.total_pages == 1);
}

#[tokio::test]
async fn paged_read_page_navigation() {
    let (ctx, _dl) = setup().await;
    seed_tasks(&ctx, 7, Priority::Medium).await;

    // Request 3 items per page — should yield 3 pages (3, 3, 1).
    let page1 = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: Some(1),
            per_page: Some(3),
        },
    )
    .await
    .expect("page 1");

    check!(page1.data.len() == 3);
    check!(page1.meta.page == 1);
    check!(page1.meta.per_page == 3);
    check!(page1.meta.total == 7);
    check!(page1.meta.total_pages == 3);

    let page2 = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: Some(2),
            per_page: Some(3),
        },
    )
    .await
    .expect("page 2");

    check!(page2.data.len() == 3);
    check!(page2.meta.page == 2);

    let page3 = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: Some(3),
            per_page: Some(3),
        },
    )
    .await
    .expect("page 3");

    check!(page3.data.len() == 1);
    check!(page3.meta.page == 3);

    // No overlap between pages.
    let all_ids: Vec<_> = page1
        .data
        .iter()
        .chain(page2.data.iter())
        .chain(page3.data.iter())
        .map(|t| t.task_id)
        .collect();
    let unique: std::collections::HashSet<_> = all_ids.iter().collect();
    check!(unique.len() == 7);
}

#[tokio::test]
async fn paged_read_beyond_last_page_returns_empty() {
    let (ctx, _dl) = setup().await;
    seed_tasks(&ctx, 3, Priority::Low).await;

    let result = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: Some(99),
            per_page: Some(10),
        },
    )
    .await
    .expect("page beyond end");

    check!(result.data.is_empty());
    check!(result.meta.page == 99);
    check!(result.meta.total == 3);
    check!(result.meta.total_pages == 1);
}

#[tokio::test]
async fn paged_read_empty_table() {
    let (ctx, _dl) = setup().await;

    let result = cinderblock_core::read::<Task, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: None,
            per_page: None,
        },
    )
    .await
    .expect("paged read on empty table");

    check!(result.data.is_empty());
    check!(result.meta.total == 0);
    // With 0 total items, total_pages should be 0.
    check!(result.meta.total_pages == 0);
}

#[tokio::test]
async fn paged_read_with_custom_config_default_per_page() {
    let (ctx, _dl) = setup().await;
    // paged_by_priority has default_per_page 3, max_per_page 5.
    seed_tasks(&ctx, 7, Priority::High).await;

    let result = cinderblock_core::read::<Task, PagedByPriority>(
        &ctx,
        &PagedByPriorityArguments {
            priority: Priority::High,
            page: None,
            per_page: None, // should default to 3
        },
    )
    .await
    .expect("paged by priority with default per_page");

    check!(result.data.len() == 3);
    check!(result.meta.per_page == 3);
    check!(result.meta.total == 7);
    check!(result.meta.total_pages == 3); // ceil(7/3) = 3
}

#[tokio::test]
async fn paged_read_max_per_page_clamping() {
    let (ctx, _dl) = setup().await;
    // paged_by_priority has max_per_page 5.
    seed_tasks(&ctx, 10, Priority::Low).await;

    let result = cinderblock_core::read::<Task, PagedByPriority>(
        &ctx,
        &PagedByPriorityArguments {
            priority: Priority::Low,
            page: Some(1),
            per_page: Some(999), // should be clamped to 5
        },
    )
    .await
    .expect("paged read with clamped per_page");

    check!(result.data.len() == 5);
    check!(result.meta.per_page == 5);
    check!(result.meta.total == 10);
    check!(result.meta.total_pages == 2);
}

#[tokio::test]
async fn paged_read_with_filter_excludes_non_matching() {
    let (ctx, _dl) = setup().await;
    seed_tasks(&ctx, 4, Priority::High).await;
    seed_tasks(&ctx, 6, Priority::Low).await;

    let result = cinderblock_core::read::<Task, PagedByPriority>(
        &ctx,
        &PagedByPriorityArguments {
            priority: Priority::High,
            page: None,
            per_page: Some(5), // max is 5
        },
    )
    .await
    .expect("paged read with filter");

    // Only the 4 High-priority tasks should appear.
    check!(result.data.len() == 4);
    check!(result.meta.total == 4);
    check!(result.meta.total_pages == 1);

    for task in &result.data {
        check!(task.priority == Priority::High);
    }
}

// ---------------------------------------------------------------------------
// # Database-generated value tests
// ---------------------------------------------------------------------------

/// Create a fresh in-memory SQLite database with the `notes` table that
/// uses an autoincrement PK and a server-side DEFAULT for `created_at`.
async fn setup_notes() -> (std::sync::Arc<cinderblock_core::Context>, SqliteDataLayer) {
    let dl = SqliteDataLayer::new("sqlite::memory:")
        .await
        .expect("connect to in-memory SQLite");

    sqlx::query(
        "CREATE TABLE notes (
            note_id INTEGER PRIMARY KEY AUTOINCREMENT,
            body TEXT NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )",
    )
    .execute(dl.pool())
    .await
    .expect("create notes table");

    let mut ctx = cinderblock_core::Context::new();
    ctx.register_data_layer(dl.clone());

    (std::sync::Arc::new(ctx), dl)
}

#[tokio::test]
async fn generated_pk_is_assigned_by_database() {
    let (ctx, _dl) = setup_notes().await;

    let note = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "Hello world".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create note");

    // The database should have assigned a positive autoincrement ID,
    // not the Rust Default::default() value of 0.
    check!(note.note_id > 0);
    check!(note.body == "Hello world");
    // created_at should be populated by the database DEFAULT, not empty.
    check!(!note.created_at.is_empty());
}

#[tokio::test]
async fn generated_pk_increments_across_inserts() {
    let (ctx, _dl) = setup_notes().await;

    let first = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "First".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create first note");

    let second = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "Second".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create second note");

    check!(second.note_id > first.note_id);
}

#[tokio::test]
async fn generated_column_not_overwritten_by_update() {
    let (ctx, _dl) = setup_notes().await;

    let created = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "Original".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create note");

    let updated = cinderblock_core::update::<Note, EditNote>(
        &created.note_id,
        EditNoteInput {
            body: "Revised".to_string(),
        },
        &ctx,
    )
    .await
    .expect("update note");

    check!(updated.body == "Revised");
    // The generated columns should be unchanged after the update.
    check!(updated.note_id == created.note_id);
    check!(updated.created_at == created.created_at);
}

#[tokio::test]
async fn read_back_generated_values_via_list() {
    let (ctx, _dl) = setup_notes().await;

    let created = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "List test".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create note");

    let notes = cinderblock_core::read::<Note, AllNotes>(&ctx, &())
        .await
        .expect("list notes");

    check!(notes.len() == 1);
    check!(notes[0].note_id == created.note_id);
    check!(notes[0].body == "List test");
    check!(notes[0].created_at == created.created_at);
}

#[tokio::test]
async fn destroy_generated_pk_resource() {
    let (ctx, _dl) = setup_notes().await;

    let created = cinderblock_core::create::<Note, AddNote>(
        AddNoteInput {
            body: "Doomed note".to_string(),
        },
        &ctx,
    )
    .await
    .expect("create note");

    let destroyed = cinderblock_core::destroy::<Note, RemoveNote>(&created.note_id, &ctx)
        .await
        .expect("destroy note");

    check!(destroyed.note_id == created.note_id);
    check!(destroyed.body == "Doomed note");

    let remaining = cinderblock_core::read::<Note, AllNotes>(&ctx, &())
        .await
        .expect("list notes");

    check!(remaining.is_empty());
}
