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
