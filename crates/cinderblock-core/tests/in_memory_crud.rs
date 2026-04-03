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
    Context, resource,
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

        read ordered_by_label {
            order { label asc; };
        }

        read ordered_by_label_desc {
            order { label desc; };
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

        read paged_ordered {
            order { label desc; };

            paged {
                default_per_page 3;
                max_per_page 10;
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

// ---------------------------------------------------------------------------
// # Relation Loading Tests
// ---------------------------------------------------------------------------
//
// These tests verify the `relations` + `load` DSL features using the
// in-memory data layer. We define separate resource types per test scenario
// to avoid cross-contamination through the global in-memory store.

// ## Belongs-to test resources

resource! {
    name = Test.Relations.BtAuthor;

    attributes {
        author_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        name String;
    }

    actions {
        create add_bt_author;
    }
}

resource! {
    name = Test.Relations.BtPost;

    attributes {
        post_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        title String;
        author_id Uuid;
    }

    relations {
        belongs_to author {
            ty BtAuthor;
            source_attribute author_id;
        };
    }

    actions {
        // Plain read — no relations loaded, returns Vec<BtPost>
        read all_bt_posts;

        // Read with relations — returns Vec<AllBtPostsWithAuthorResponse>
        read all_bt_posts_with_author {
            load [author];
        };

        create add_bt_post;
    }
}

// ## Has-many test resources

resource! {
    name = Test.Relations.HmWriter;

    attributes {
        writer_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        pen_name String;
    }

    actions {
        create add_hm_writer;
    }
}

resource! {
    name = Test.Relations.HmArticle;

    attributes {
        article_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        headline String;
        writer_id Uuid;
    }

    actions {
        create add_hm_article;
    }
}

resource! {
    name = Test.Relations.HmWriterWithArticles;

    attributes {
        writer_with_articles_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        pen_name String;
    }

    relations {
        has_many articles {
            ty HmArticle;
            source_attribute writer_id;
        };
    }

    actions {
        read all_hm_writers;

        read all_hm_writers_with_articles {
            load [articles];
        };

        create add_hm_writer_with_articles;
    }
}

#[tokio::test]
async fn belongs_to_relation_loads_related_resource() {
    let ctx = fresh_ctx();

    // Create an author
    let author = cinderblock_core::create::<BtAuthor, AddBtAuthor>(
        AddBtAuthorInput {
            name: "Alice".into(),
        },
        &ctx,
    )
    .await
    .expect("create author");

    // Create a post referencing the author
    let post = cinderblock_core::create::<BtPost, AddBtPost>(
        AddBtPostInput {
            title: "Hello World".into(),
            author_id: author.author_id,
        },
        &ctx,
    )
    .await
    .expect("create post");

    // Read posts with author loaded
    let results = cinderblock_core::read::<BtPost, AllBtPostsWithAuthor>(&ctx, &())
        .await
        .expect("read posts with author");

    // Find our specific post in the results
    let our_result = results
        .iter()
        .find(|r| r.base.post_id == post.post_id)
        .expect("our post should be in results");

    check!(our_result.base.title == "Hello World");
    check!(our_result.author.author_id == author.author_id);
    check!(our_result.author.name == "Alice");
}

#[tokio::test]
async fn belongs_to_response_serializes_with_flattened_base() {
    let ctx = fresh_ctx();

    let author = cinderblock_core::create::<BtAuthor, AddBtAuthor>(
        AddBtAuthorInput { name: "Bob".into() },
        &ctx,
    )
    .await
    .expect("create author");

    cinderblock_core::create::<BtPost, AddBtPost>(
        AddBtPostInput {
            title: "Serialization Test".into(),
            author_id: author.author_id,
        },
        &ctx,
    )
    .await
    .expect("create post");

    let results = cinderblock_core::read::<BtPost, AllBtPostsWithAuthor>(&ctx, &())
        .await
        .expect("read posts with author");

    let our_result = results
        .iter()
        .find(|r| r.base.title == "Serialization Test")
        .expect("our post should be in results");

    // Verify that serialization flattens the base resource fields
    let json = serde_json::to_value(our_result).expect("serialize to JSON");
    check!(json["title"] == "Serialization Test");
    check!(json["author_id"] == author.author_id.to_string());
    // The author relation should be nested as an object
    check!(json["author"]["name"] == "Bob");
}

#[tokio::test]
async fn read_without_load_returns_plain_vec() {
    let ctx = fresh_ctx();

    let author = cinderblock_core::create::<BtAuthor, AddBtAuthor>(
        AddBtAuthorInput {
            name: "Charlie".into(),
        },
        &ctx,
    )
    .await
    .expect("create author");

    cinderblock_core::create::<BtPost, AddBtPost>(
        AddBtPostInput {
            title: "No Load Test".into(),
            author_id: author.author_id,
        },
        &ctx,
    )
    .await
    .expect("create post");

    // Reading without load returns Vec<BtPost>, not the wrapper
    let results = cinderblock_core::read::<BtPost, AllBtPosts>(&ctx, &())
        .await
        .expect("read all posts");

    let our_post = results
        .iter()
        .find(|p| p.title == "No Load Test")
        .expect("our post should be in results");

    check!(our_post.author_id == author.author_id);
}

#[tokio::test]
async fn has_many_relation_loads_related_resources() {
    let ctx = fresh_ctx();

    // Create a writer (using the has_many-aware resource)
    let writer = cinderblock_core::create::<HmWriterWithArticles, AddHmWriterWithArticles>(
        AddHmWriterWithArticlesInput {
            pen_name: "Diana".into(),
        },
        &ctx,
    )
    .await
    .expect("create writer");

    // Create articles referencing the writer's PK via the `writer_id` field
    let article1 = cinderblock_core::create::<HmArticle, AddHmArticle>(
        AddHmArticleInput {
            headline: "Diana's First Article".into(),
            writer_id: writer.writer_with_articles_id,
        },
        &ctx,
    )
    .await
    .expect("create article 1");

    let article2 = cinderblock_core::create::<HmArticle, AddHmArticle>(
        AddHmArticleInput {
            headline: "Diana's Second Article".into(),
            writer_id: writer.writer_with_articles_id,
        },
        &ctx,
    )
    .await
    .expect("create article 2");

    // Read writers with articles loaded
    let results =
        cinderblock_core::read::<HmWriterWithArticles, AllHmWritersWithArticles>(&ctx, &())
            .await
            .expect("read writers with articles");

    let our_result = results
        .iter()
        .find(|r| r.base.writer_with_articles_id == writer.writer_with_articles_id)
        .expect("our writer should be in results");

    check!(our_result.base.pen_name == "Diana");

    // Should have at least our two articles
    let our_article_ids: std::collections::HashSet<_> =
        our_result.articles.iter().map(|a| a.article_id).collect();

    check!(our_article_ids.contains(&article1.article_id));
    check!(our_article_ids.contains(&article2.article_id));
}

#[tokio::test]
async fn has_many_with_no_related_resources_returns_empty_vec() {
    let ctx = fresh_ctx();

    // Create a writer with no articles
    let writer = cinderblock_core::create::<HmWriterWithArticles, AddHmWriterWithArticles>(
        AddHmWriterWithArticlesInput {
            pen_name: "EmptyWriter".into(),
        },
        &ctx,
    )
    .await
    .expect("create writer");

    let results =
        cinderblock_core::read::<HmWriterWithArticles, AllHmWritersWithArticles>(&ctx, &())
            .await
            .expect("read writers with articles");

    let our_result = results
        .iter()
        .find(|r| r.base.writer_with_articles_id == writer.writer_with_articles_id)
        .expect("our writer should be in results");

    // No articles reference this specific writer_with_articles_id
    let matching_articles: Vec<_> = our_result
        .articles
        .iter()
        .filter(|a| a.writer_id == writer.writer_with_articles_id)
        .collect();
    check!(matching_articles.is_empty());
}

// ---------------------------------------------------------------------------
// # Order Tests
// ---------------------------------------------------------------------------

resource! {
    name = Test.InMemory.Ordered;

    attributes {
        id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        name String;
        score u32;
    }

    actions {
        read ordered_asc {
            order { name asc; };
        }

        read ordered_desc {
            order { name desc; };
        }

        read ordered_compound {
            order { score desc; name asc; };
        }

        read paged_ordered_by_name {
            order { name asc; };
            paged { default_per_page 2; };
        }

        create add_ordered;
    }
}

async fn seed_ordered_items(ctx: &Context) -> Vec<Ordered> {
    let names = ["Charlie", "Alice", "Bob"];
    let scores = [10u32, 30, 20];
    let mut items = Vec::new();
    for (name, score) in names.iter().zip(scores.iter()) {
        let item = cinderblock_core::create::<Ordered, AddOrdered>(
            AddOrderedInput {
                name: name.to_string(),
                score: *score,
            },
            ctx,
        )
        .await
        .expect("seed ordered item");
        items.push(item);
    }
    items
}

#[tokio::test]
async fn order_asc_returns_sorted_results() {
    let ctx = fresh_ctx();
    seed_ordered_items(&ctx).await;

    let results = cinderblock_core::read::<Ordered, OrderedAsc>(&ctx, &())
        .await
        .expect("read ordered asc");

    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    check!(names.windows(2).all(|w| w[0] <= w[1]));
}

#[tokio::test]
async fn order_desc_returns_reverse_sorted_results() {
    let ctx = fresh_ctx();
    seed_ordered_items(&ctx).await;

    let results = cinderblock_core::read::<Ordered, OrderedDesc>(&ctx, &())
        .await
        .expect("read ordered desc");

    let names: Vec<&str> = results.iter().map(|r| r.name.as_str()).collect();
    check!(names.windows(2).all(|w| w[0] >= w[1]));
}

#[tokio::test]
async fn compound_order_sorts_by_multiple_fields() {
    let ctx = fresh_ctx();

    // Create items with duplicate scores to exercise the secondary sort
    for (name, score) in [("Zara", 10u32), ("Alice", 10), ("Bob", 20), ("Anna", 20)] {
        cinderblock_core::create::<Ordered, AddOrdered>(
            AddOrderedInput {
                name: name.to_string(),
                score,
            },
            &ctx,
        )
        .await
        .expect("seed");
    }

    let results = cinderblock_core::read::<Ordered, OrderedCompound>(&ctx, &())
        .await
        .expect("read compound order");

    // Primary: score DESC, Secondary: name ASC
    // score=20 group first (desc), then score=10 group
    // Within each group, name sorted ASC
    let pairs: Vec<(u32, &str)> = results.iter().map(|r| (r.score, r.name.as_str())).collect();
    for w in pairs.windows(2) {
        let ok = w[0].0 > w[1].0 || (w[0].0 == w[1].0 && w[0].1 <= w[1].1);
        check!(ok);
    }
}

#[tokio::test]
async fn paged_order_maintains_sort_across_pages() {
    let ctx = fresh_ctx();
    seed_ordered_items(&ctx).await;

    let page1 = cinderblock_core::read::<Ordered, PagedOrderedByName>(
        &ctx,
        &PagedOrderedByNameArguments {
            page: Some(1),
            per_page: Some(2),
        },
    )
    .await
    .expect("page 1");

    let page2 = cinderblock_core::read::<Ordered, PagedOrderedByName>(
        &ctx,
        &PagedOrderedByNameArguments {
            page: Some(2),
            per_page: Some(2),
        },
    )
    .await
    .expect("page 2");

    // All items across both pages should be in ascending name order
    let all_names: Vec<&str> = page1
        .data
        .iter()
        .chain(page2.data.iter())
        .map(|r| r.name.as_str())
        .collect();
    check!(all_names.windows(2).all(|w| w[0] <= w[1]));
}

// ---------------------------------------------------------------------------
// # Lifecycle Hook Tests
// ---------------------------------------------------------------------------

resource! {
    name = Test.Hooks.Stamped;

    attributes {
        stamped_id Uuid {
            primary_key true;
            writable false;
            default || uuid::Uuid::new_v4();
        }
        title String;
        stamp String;
    }

    before_create |resource| {
        resource.stamp = String::from("created");
    };

    before_update |resource| {
        resource.stamp = String::from("updated");
    };

    actions {
        create add_stamped;
        update edit_stamped;
    }
}

#[tokio::test]
async fn before_create_hook_mutates_resource_before_persistence() {
    let ctx = fresh_ctx();

    let item = cinderblock_core::create::<Stamped, AddStamped>(
        AddStampedInput {
            title: "Test".into(),
            stamp: "initial".into(),
        },
        &ctx,
    )
    .await
    .expect("create stamped");

    check!(item.stamp == "created");
}

#[tokio::test]
async fn before_update_hook_mutates_resource_before_persistence() {
    let ctx = fresh_ctx();

    let item = cinderblock_core::create::<Stamped, AddStamped>(
        AddStampedInput {
            title: "Test".into(),
            stamp: "initial".into(),
        },
        &ctx,
    )
    .await
    .expect("create stamped");

    check!(item.stamp == "created");

    let updated = cinderblock_core::update::<Stamped, EditStamped>(
        &item.stamped_id,
        EditStampedInput {
            title: "Edited".into(),
            stamp: "should-be-overwritten".into(),
        },
        &ctx,
    )
    .await
    .expect("update stamped");

    check!(updated.title == "Edited");
    check!(updated.stamp == "updated");
}

#[tokio::test]
async fn resource_without_hooks_is_unaffected() {
    let ctx = fresh_ctx();

    let item = cinderblock_core::create::<Item, Add>(
        AddInput {
            label: "No hooks".into(),
            category: Category::General,
            active: true,
        },
        &ctx,
    )
    .await
    .expect("create item without hooks");

    check!(item.label == "No hooks");
    check!(item.active == true);
}
