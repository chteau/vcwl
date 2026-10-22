//! Server functions: compiled with the app, run by `vcwl::server::Functions`.

use std::path::Path;

use vcwl::compile::{Options, compile};
use vcwl::diag::SourceMap;
use vcwl::server::{Functions, Reply};

const SRC: &str = r#"
app "T"

struct Item {
    id: Int,
    name: String,
    price: Float,
}

server fn add(a: Int, b: Int) -> Int {
    a + b
}

server fn put(item: Item) -> [Item] {
    let all = load<[Item]>("items").unwrap_or([]).push(item)
    save("items", all)
    all
}

server fn count() -> Int {
    load<[Item]>("items").unwrap_or([]).len()
}

server fn home() -> String {
    env("VCWL_TEST_NEVER_SET").unwrap_or("unset") + " " + "server-only-marker"
}

server fn forget() {
    remove("items")
}

server fn spin() -> Int {
    var n = 0
    while true {
        n += 1
    }
    n
}

component App {
    state n: Int = 0

    button("{n}") {
        on click {
            fetch add(n, 1) {
                Ok(v) => {
                    n = v
                }
                Err(_) => {}
            }
        }
    }
}
"#;

fn setup(name: &str) -> (Vec<u8>, Vec<u8>, std::path::PathBuf) {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.vcwl"), SRC).unwrap();
    let out = compile(
        &dir.join("main.vcwl"),
        &mut SourceMap::default(),
        &Options { dev: false },
    )
    .unwrap_or_else(|f| panic!("{:?}", f.diags.iter().map(|d| &d.msg).collect::<Vec<_>>()));
    (out.wasm, out.server.expect("a server module"), dir)
}

#[test]
fn server_functions_run_with_a_persistent_store() {
    let (client, server, dir) = setup("server-store");
    let store = dir.join("data/store.json");
    let fns = Functions::load(&server, store.clone()).unwrap();
    assert_eq!(
        fns.names(),
        ["add", "put", "count", "home", "forget", "spin"]
    );
    assert_eq!(fns.call("add", "[2, 40]"), Reply::Ok("42".into()));
    let item = r#"[{"id": 1, "name": "Lamp \"x\"", "price": 9.5}]"#;
    assert_eq!(
        fns.call("put", item),
        Reply::Ok(r#"[{"id":1,"name":"Lamp \"x\"","price":9.5}]"#.into())
    );
    assert!(
        std::fs::read_to_string(&store)
            .unwrap()
            .contains("\"items\": [{\"id\":1")
    );
    // A new runtime reads the same store.
    let fns = Functions::load(&server, store.clone()).unwrap();
    assert_eq!(fns.call("count", "[]"), Reply::Ok("1".into()));
    assert_eq!(fns.call("forget", "[]"), Reply::Ok("null".into()));
    assert_eq!(fns.call("count", "[]"), Reply::Ok("0".into()));
    assert_eq!(
        fns.call("home", "[]"),
        Reply::Ok("\"unset server-only-marker\"".into())
    );
    // The client module calls server functions by URL only.
    let text = String::from_utf8_lossy(&client);
    assert!(text.contains("__vcwl/fn/add") && !text.contains("server-only-marker"));
}

#[test]
fn bad_calls_are_refused() {
    let (_, server, dir) = setup("server-bad");
    let fns = Functions::load(&server, dir.join("store.json")).unwrap();
    assert_eq!(fns.call("nope", "[]"), Reply::NotFound);
    assert!(matches!(fns.call("add", "[1]"), Reply::BadRequest(m) if m.contains("2 arguments")));
    assert!(
        matches!(fns.call("add", "[1, \"2\"]"), Reply::BadRequest(m) if m.starts_with("argument 2"))
    );
    assert!(matches!(fns.call("add", "{"), Reply::BadRequest(_)));
    assert!(
        matches!(fns.call("put", "[{\"id\": 1}]"), Reply::BadRequest(m) if m.contains(".name"))
    );
    let start = std::time::Instant::now();
    assert!(matches!(fns.call("spin", "[]"), Reply::Failed(m) if m.contains("time limit")));
    assert!(start.elapsed().as_secs() < 10);
    assert_eq!(
        fns.call("add", "[1, 1]"),
        Reply::Ok("2".into()),
        "still serving"
    );
}
