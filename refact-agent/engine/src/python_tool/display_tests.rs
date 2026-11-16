use super::*;
use serde_json::json;

fn show(value: Value, kwargs: Value) -> Result<Normalized, String> {
    normalize(&json!([value]), &kwargs, 1 << 20)
}
fn table(value: Value) -> (Vec<String>, Vec<Vec<Value>>) {
    let d = show(value, json!({})).unwrap();
    (d.columns, d.rows)
}

#[test]
fn list_of_dicts_uses_first_seen_keys_and_nulls() {
    let (c, r) = table(json!([{"a":1,"b":2},{"c":3,"a":4}]));
    assert_eq!(c, ["a", "b", "c"]);
    assert_eq!(
        r,
        vec![
            vec![json!(1), json!(2), Value::Null],
            vec![json!(4), Value::Null, json!(3)]
        ]
    );
    let d = show(json!([{"a":1,"b":2},{"c":3}]), json!({"columns":["c","b"]})).unwrap();
    assert_eq!(
        (d.columns, d.rows),
        (
            vec!["c".to_string(), "b".into()],
            vec![vec![Value::Null, json!(2)], vec![json!(3), Value::Null]]
        )
    );
}

#[test]
fn unknown_names_in_columns_raise_for_every_selecting_form() {
    for (value, known) in [
        (json!([{"a":1,"b":2}]), "a, b"),
        (json!([1, 2, 3]), "value"),
        (json!({"name":["a"],"n":[1]}), "name, n"),
        (json!({"x":1,"y":"two"}), "key, value"),
    ] {
        let error = show(value.clone(), json!({"columns":["zz"]})).unwrap_err();
        assert_eq!(
            error,
            format!("show(): column 'zz' does not exist; columns are {known}"),
            "{value}"
        );
    }
}

#[test]
fn list_of_lists_pads_and_names_columns() {
    let (c, r) = table(json!([[1, "x"], [2], [3, "y", true]]));
    assert_eq!(c, ["col1", "col2", "col3"]);
    assert_eq!(r[1], vec![json!(2), Value::Null, Value::Null]);
    let d = show(json!([[1, 2], [3]]), json!({"columns":["n","m"]})).unwrap();
    assert_eq!(d.columns, ["n", "m"]);
    assert_eq!(d.rows[1], vec![json!(3), Value::Null]);
}

#[test]
fn dict_forms_and_scalar_lists() {
    let (c, r) = table(json!({"name":["a","b","c"],"n":[1,2]}));
    assert_eq!(c, ["name", "n"]);
    assert_eq!(r[2], vec![json!("c"), Value::Null]);
    let d = show(json!({"name":["a"],"n":[1]}), json!({"columns":["n"]})).unwrap();
    assert_eq!(
        (d.columns, d.rows),
        (vec!["n".to_string()], vec![vec![json!(1)]])
    );
    let (c, r) = table(json!({"x":1,"y":"two"}));
    assert_eq!(c, ["key", "value"]);
    assert_eq!(
        r,
        vec![vec![json!("x"), json!(1)], vec![json!("y"), json!("two")]]
    );
    let (c, r) = table(json!([1, "a", null]));
    assert_eq!(c, ["value"]);
    assert_eq!(r.len(), 3);
    let d = show(json!([]), json!({})).unwrap();
    assert_eq!((d.rows.len(), d.rows_total), (0, 0));
}

#[test]
fn rejected_forms_name_the_accepted_ones() {
    for bad in [
        json!({"a":[1],"b":2}),
        json!(5),
        json!([[1], {"a":1}]),
        json!({"__repr__":"<object>"}),
    ] {
        let error = show(bad.clone(), json!({})).unwrap_err();
        assert!(
            error.contains("list of dicts") && error.contains("dict of lists"),
            "{bad}: {error}"
        );
    }
}

#[test]
fn cells_are_json_scalars() {
    let (_, r) = table(
        json!([[{"__repr__":"nan"}, {"__repr__":"inf"}, {"__repr__":"-inf"}, [1, 2], {"k":"v"}, {"__repr__":"<function f>"}, 1.5, false]]),
    );
    assert_eq!(r[0][..3], [Value::Null, Value::Null, Value::Null]);
    assert_eq!(r[0][3], json!("[1,2]"));
    assert_eq!(r[0][4], json!("{\"k\":\"v\"}"));
    assert_eq!(r[0][5], json!("<function f>"));
    assert_eq!(r[0][6..], [json!(1.5), json!(false)]);
}

#[test]
fn kind_inference_and_errors() {
    assert_eq!(show(json!("# hi"), json!({})).unwrap().kind, "markdown");
    assert_eq!(
        show(json!("# hi"), json!({})).unwrap().text.as_deref(),
        Some("# hi")
    );
    assert_eq!(show(json!([1]), json!({})).unwrap().kind, "table");
    let e = show(json!([1]), json!({"kind":"heatmap"})).unwrap_err();
    assert!(
        e.contains("table, markdown, bar, line, area, scatter, pie"),
        "{e}"
    );
    assert!(show(json!([1]), json!({"kind":"markdown"}))
        .unwrap_err()
        .contains("str"));
    assert!(normalize(&json!([]), &json!({}), 100).is_err());
    assert!(normalize(&json!([1]), &json!({"value":1}), 100).is_err());
    assert!(normalize(&json!([1, 2]), &json!({}), 100).is_err());
    assert!(normalize(&json!([1]), &json!({"color":"red"}), 100).is_err());
    assert_eq!(
        normalize(&json!([]), &json!({"value":"t"}), 100)
            .unwrap()
            .kind,
        "markdown"
    );
    let wide: Vec<Value> = vec![json!((0..201).collect::<Vec<_>>())];
    assert!(show(json!(wide), json!({})).unwrap_err().contains("200"));
    let title = show(json!([1]), json!({"title": "t".repeat(300)}))
        .unwrap()
        .title
        .unwrap();
    assert_eq!(title.chars().count(), 120);
    let d = show(json!([{"a":"x","b":1}]), json!({"x":"zz","y":"zz"})).unwrap();
    assert_eq!((d.x, d.y.len()), (None, 0));
}

#[test]
fn chart_defaults_and_column_errors() {
    let rows = json!([{"m":"a","f":1,"g":2.5,"s":"x","b":true},{"m":"b","f":null,"g":3,"s":"y","b":false}]);
    let d = show(rows.clone(), json!({"kind":"bar"})).unwrap();
    assert_eq!(
        (d.x.as_deref(), d.y.clone()),
        (Some("m"), vec!["f".to_string(), "g".into()])
    );
    let d = show(rows.clone(), json!({"kind":"pie"})).unwrap();
    assert_eq!(d.y, ["f"]);
    let d = show(rows.clone(), json!({"kind":"line","x":"g","y":"f"})).unwrap();
    assert_eq!(
        (d.x.as_deref(), d.y.clone()),
        (Some("g"), vec!["f".to_string()])
    );
    assert!(show(rows.clone(), json!({"kind":"bar","x":"nope"}))
        .unwrap_err()
        .contains("'nope'"));
    assert!(show(rows.clone(), json!({"kind":"bar","y":["f","nope"]}))
        .unwrap_err()
        .contains("'nope'"));
    assert!(show(rows.clone(), json!({"kind":"bar","y":"s"}))
        .unwrap_err()
        .contains("numeric"));
    assert!(show(rows.clone(), json!({"kind":"bar","y":"b"}))
        .unwrap_err()
        .contains("numeric"));
    assert!(show(json!([{"a":"x","b":"y"}]), json!({"kind":"scatter"}))
        .unwrap_err()
        .contains("numeric"));
    let mixed = json!([["a", 1], ["b", "n/a"], ["c", 3], ["d", null]]);
    let error = show(mixed.clone(), json!({"kind":"bar","y":"col2"})).unwrap_err();
    assert!(
        error.contains("y column 'col2'") && error.contains("numbers"),
        "{error}"
    );
    let error = show(mixed, json!({"kind":"bar","y":["col1"]})).unwrap_err();
    assert!(error.contains("y column 'col1'"), "{error}");
    let d = show(
        json!([["a", 1], ["b", null], ["c", 3]]),
        json!({"kind":"bar","y":"col2"}),
    )
    .unwrap();
    assert_eq!(d.y, ["col2"]);
}

#[test]
fn file_caps_keep_10000_rows_and_the_real_total() {
    let rows: Vec<Value> = (0..12_000).map(|i| json!([i, "x".repeat(10)])).collect();
    let d = show(json!(rows), json!({"title":"Big"})).unwrap();
    assert_eq!(
        (d.rows.len(), d.rows_total, d.truncated),
        (10_000, 12_000, true)
    );
    let file = file_json(&d, 3);
    assert_eq!(file["rows"].as_array().unwrap().len(), 10_000);
    assert_eq!(
        (
            file["rows_total"].clone(),
            file["schema"].clone(),
            file["index"].clone()
        ),
        (json!(12_000), json!(1), json!(3))
    );
    assert!(file.get("text").is_none());
    let long = show(json!([["y".repeat(20_000)]]), json!({})).unwrap();
    assert_eq!(long.rows[0][0].as_str().unwrap().chars().count(), 10_000);
    let md = normalize(&json!(["é".repeat(100)]), &json!({}), 11).unwrap();
    assert_eq!((md.text.unwrap(), md.truncated), ("é".repeat(5), true));
}

#[test]
fn inline_caps_for_tables_charts_cells_and_markdown() {
    let path = std::path::Path::new("/w/displays/1.json");
    let rows: Vec<Value> = (0..600).map(|i| json!([i, i * 2])).collect();
    let t = inline(&show(json!(rows.clone()), json!({})).unwrap(), 1, path);
    assert_eq!(
        (t.rows.as_ref().unwrap().len(), t.rows_total, t.truncated),
        (200, Some(600), true)
    );
    let c = inline(&show(json!(rows), json!({"kind":"line"})).unwrap(), 1, path);
    assert_eq!(
        (c.rows.as_ref().unwrap().len(), c.rows_total, c.truncated),
        (500, Some(600), true)
    );
    let small = inline(&show(json!([[1, 2]]), json!({})).unwrap(), 1, path);
    assert_eq!((small.rows_total, small.truncated), (None, false));
    assert_eq!(small.path, "/w/displays/1.json");
    let cell = inline(
        &show(json!([["z".repeat(500)]]), json!({})).unwrap(),
        1,
        path,
    );
    let text = cell.rows.unwrap()[0][0].as_str().unwrap().to_owned();
    assert_eq!(text.chars().count(), 200);
    assert!(text.ends_with('…') && cell.truncated && cell.rows_total.is_none());
    let wide = inline(
        &show(
            json!([(0..40).collect::<Vec<_>>()]),
            json!({"kind":"bar","x":"col1","y":"col40"}),
        )
        .unwrap(),
        1,
        path,
    );
    assert_eq!(wide.columns.len(), 30);
    assert!(wide.columns.contains(&"col40".to_string()) && wide.truncated);
    assert_eq!(wide.y, ["col40"]);
    let md = inline(
        &show(json!("m".repeat(25_000)), json!({})).unwrap(),
        1,
        path,
    );
    assert_eq!(
        (md.text.unwrap().chars().count(), md.truncated, md.rows),
        (20_000, true, None)
    );
    let total = inline(
        &show(json!((0..12_000).collect::<Vec<_>>()), json!({})).unwrap(),
        1,
        path,
    );
    assert_eq!(total.rows_total, Some(12_000));
}

#[test]
fn notes_describe_shape_without_data() {
    let t = show(
        json!([{"name":"secret_cell","n":1}]),
        json!({"title":"Slowest tests"}),
    )
    .unwrap();
    assert_eq!(
        note_line(&t),
        "shown to the user: table \"Slowest tests\" (1 row × 2 columns)"
    );
    let c = show(
        json!([{"module":"a","failures":1,"flaky":2}]),
        json!({"kind":"bar","title":"Failures by module"}),
    )
    .unwrap();
    assert_eq!(
        note_line(&c),
        "shown to the user: bar chart \"Failures by module\" (1 row; x=module; y=failures, flaky)"
    );
    let m = show(json!("ab"), json!({"title":"Notes"})).unwrap();
    assert_eq!(
        note_line(&m),
        "shown to the user: markdown \"Notes\" (2 characters)"
    );
    assert!(!note_line(&t).contains("secret_cell"));
}

#[tokio::test]
async fn log_limits_inline_count_and_bytes_but_counts_everything() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = DisplayLog::default();
    for i in 0..14 {
        let shown = log
            .show(dir.path(), &json!([[i]]), &json!({}), 1 << 20)
            .await
            .unwrap();
        assert_eq!(shown.is_some(), i < 12);
    }
    assert_eq!((log.total, log.inline.len(), log.truncated), (14, 12, true));
    assert!(dir.path().join("displays/14.json").exists());
    let text = log.model_text();
    assert_eq!(text.lines().count(), 13);
    assert!(text.ends_with("… and 2 more"));
    let mut extra = json!({});
    log.extend_extra(&mut extra);
    assert_eq!(
        (
            extra["displays_total"].clone(),
            extra["displays_truncated"].clone()
        ),
        (json!(14), json!(true))
    );
    assert_eq!(extra["displays"].as_array().unwrap().len(), 12);

    let mut log = DisplayLog::default();
    let big: Vec<Value> = (0..200).map(|_| json!(vec!["w".repeat(199); 10])).collect();
    let mut kept = 0;
    for _ in 0..3 {
        if log
            .show(dir.path(), &json!([big.clone()]), &json!({}), 1 << 20)
            .await
            .unwrap()
            .is_some()
        {
            kept += 1;
        }
    }
    assert_eq!((kept, log.total, log.truncated), (1, 3, true));
    assert!(log.inline_bytes <= RUN_INLINE_BYTES);

    let mut empty = json!({});
    DisplayLog::default().extend_extra(&mut empty);
    assert_eq!(empty, json!({}));
    assert!(log
        .show(dir.path(), &json!([5]), &json!({}), 100)
        .await
        .is_err());
    assert_eq!(log.total, 3);
}

#[test]
fn files_read_back_with_inline_caps() {
    let d = show(json!((0..300).collect::<Vec<_>>()), json!({"title":"T"})).unwrap();
    let (index, back) = from_file(&file_json(&d, 4)).unwrap();
    assert_eq!((index, &back), (4, &d));
    assert!(from_file(&json!({"kind":"table"})).is_none());
}

#[tokio::test]
async fn show_calls_and_display_file_bytes_are_bounded_per_run() {
    let dir = tempfile::tempdir().unwrap();
    let mut log = DisplayLog::default();
    for i in 0..RUN_MAX_SHOWS {
        log.show(dir.path(), &json!([[i]]), &json!({}), 1 << 20)
            .await
            .unwrap();
    }
    match log
        .show(dir.path(), &json!([[1]]), &json!({}), 1 << 20)
        .await
    {
        Err(ShowError::Limit(message)) => {
            assert!(
                message.contains("limit for this run is reached"),
                "{message}"
            )
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(log.total, RUN_MAX_SHOWS);
    let mut log = DisplayLog {
        file_bytes: RUN_MAX_FILE_BYTES - 10,
        ..Default::default()
    };
    match log
        .show(dir.path(), &json!([[1, 2, 3]]), &json!({}), 1 << 20)
        .await
    {
        Err(ShowError::Limit(message)) => assert!(message.contains("MiB"), "{message}"),
        other => panic!("{other:?}"),
    }
    assert_eq!(log.total, 0);
    assert!(matches!(
        log.show(dir.path(), &json!([5]), &json!({"kind":"nope"}), 100)
            .await,
        Err(ShowError::Type(_))
    ));
}
