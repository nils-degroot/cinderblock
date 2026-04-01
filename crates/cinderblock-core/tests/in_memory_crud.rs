// # In-Memory Data Layer Integration Tests
//
// End-to-end tests that verify the full pipeline:
//   resource! macro → InMemoryDataLayer CRUD
//
// Each test uses the default InMemoryDataLayer backed by a global
// `LazyLock<Arc<RwLock<HashMap>>>` per resource type, so tests that share
// a resource type see each other's data. To keep tests independent we use
// a dedicated resource type defined here.

use assert2::check;
use cinderblock_core::{
    resource, Context,
    serde::{Deserialize, Serialize},
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// # Test Resource Definition
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
enum Category {
    #[default]
    General,
    Urgent,
    Archive,
}

resource! {
    name = Test.InMemory.Item;

    attributes {
        item_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        label String;
        category Category;
        active bool;
    }

    actions {
        read all;

        read by_category {
            argument { category: Category };
            filter { category == arg(category) };
        }

        read paged_all {
            paged;
        }

        read paged_by_category {
            argument { category: Category };
            filter { category == arg(category) };

            paged {
                default_per_page 3;
                max_per_page 5;
            };
        }

        create add;

        destroy remove;
    }
}

// ---------------------------------------------------------------------------
// # Helpers
// ---------------------------------------------------------------------------

fn fresh_ctx() -> Context {
    Context::new()
}

async fn seed_items(ctx: &Context, n: usize, category: Category) -> Vec<Item> {
    let mut items = Vec::with_capacity(n);
    for i in 0..n {
        let item = cinderblock_core::create::<Item, Add>(
            AddInput {
                label: format!("Item {i}"),
                category: category.clone(),
                active: true,
            },
            ctx,
        )
        .await
        .expect("seed item");
        items.push(item);
    }
    items
}

// ---------------------------------------------------------------------------
// # Non-Paged Baseline
// ---------------------------------------------------------------------------

#[tokio::test]
async fn non_paged_read_returns_vec() {
    let ctx = fresh_ctx();
    seed_items(&ctx, 3, Category::General).await;

    let items = cinderblock_core::read::<Item, All>(&ctx, &())
        .await
        .expect("list items");

    // The in-memory store is global, so we may have items from other tests.
    // Just verify we got at least the 3 we inserted.
    check!(items.len() >= 3);
}

// ---------------------------------------------------------------------------
// # Paged Read Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn paged_read_returns_paginated_result() {
    let ctx = fresh_ctx();
    // Seed enough items to be meaningful. Since the in-memory store is global,
    // we rely on total being >= what we seeded.
    seed_items(&ctx, 5, Category::General).await;

    let result = cinderblock_core::read::<Item, PagedAll>(
        &ctx,
        &PagedAllArguments {
            page: None,
            per_page: None,
        },
    )
    .await
    .expect("paged read all");

    // Default per_page is DEFAULT_PER_PAGE (100), so all items fit on one page.
    check!(result.meta.page == 1);
    check!(result.meta.per_page == cinderblock_core::DEFAULT_PER_PAGE);
    check!(result.meta.total >= 5);
    check!(result.data.len() as u64 <= result.meta.total);
}

#[tokio::test]
async fn paged_read_page_navigation() {
    let ctx = fresh_ctx();
    seed_items(&ctx, 7, Category::Urgent).await;

    // Use a filter to isolate our items from global state, then paginate.
    let page1 = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Urgent,
            page: Some(1),
            per_page: Some(3), // default_per_page is 3, max is 5
        },
    )
    .await
    .expect("page 1");

    check!(page1.data.len() == 3);
    check!(page1.meta.page == 1);
    check!(page1.meta.per_page == 3);
    check!(page1.meta.total >= 7);

    let page2 = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Urgent,
            page: Some(2),
            per_page: Some(3),
        },
    )
    .await
    .expect("page 2");

    check!(page2.data.len() == 3);
    check!(page2.meta.page == 2);

    let page3 = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Urgent,
            page: Some(3),
            per_page: Some(3),
        },
    )
    .await
    .expect("page 3");

    // At least 1 item on page 3 (we seeded 7 Urgent items, ceil(7/3)=3 pages).
    check!(page3.data.len() >= 1);
    check!(page3.meta.page == 3);

    // No overlap between page 1 and page 2.
    let ids_1: std::collections::HashSet<_> = page1.data.iter().map(|i| i.item_id).collect();
    let ids_2: std::collections::HashSet<_> = page2.data.iter().map(|i| i.item_id).collect();
    check!(ids_1.is_disjoint(&ids_2));
}

#[tokio::test]
async fn paged_read_beyond_last_page_returns_empty() {
    let ctx = fresh_ctx();

    let result = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Archive,
            page: Some(999),
            per_page: Some(5),
        },
    )
    .await
    .expect("page beyond end");

    check!(result.data.is_empty());
    check!(result.meta.page == 999);
}

#[tokio::test]
async fn paged_read_custom_default_per_page() {
    let ctx = fresh_ctx();
    // Seed items with a unique category to isolate from other tests.
    seed_items(&ctx, 10, Category::Archive).await;

    // paged_by_category has default_per_page 3, max_per_page 5.
    let result = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Archive,
            page: None,
            per_page: None, // should default to 3
        },
    )
    .await
    .expect("paged read with default per_page");

    check!(result.data.len() == 3);
    check!(result.meta.per_page == 3);
}

#[tokio::test]
async fn paged_read_max_per_page_clamping() {
    let ctx = fresh_ctx();
    seed_items(&ctx, 10, Category::Archive).await;

    // paged_by_category has max_per_page 5 — requesting 999 should clamp to 5.
    let result = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Archive,
            page: Some(1),
            per_page: Some(999),
        },
    )
    .await
    .expect("paged read with clamped per_page");

    check!(result.data.len() == 5);
    check!(result.meta.per_page == 5);
}

#[tokio::test]
async fn paged_read_total_pages_calculation() {
    let ctx = fresh_ctx();
    seed_items(&ctx, 10, Category::Archive).await;

    let result = cinderblock_core::read::<Item, PagedByCategory>(
        &ctx,
        &PagedByCategoryArguments {
            category: Category::Archive,
            page: Some(1),
            per_page: Some(4), // clamped to 4 (< max 5)
        },
    )
    .await
    .expect("check total_pages");

    // total_pages = ceil(total / per_page). With >=10 Archive items and
    // per_page=4, total_pages >= 3.
    let expected_pages = (result.meta.total as f64 / 4.0).ceil() as u32;
    check!(result.meta.total_pages == expected_pages);
}
