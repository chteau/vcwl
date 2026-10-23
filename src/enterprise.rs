//! `vcwl new --enterprise`: a small counter, split across as many layers as the language allows.

use std::path::Path;

const ENTITY: &str = r#"export struct CounterEntity {
    value: Int,
}

export interface Describable {
    fn describe(self) -> String
}

impl Describable for CounterEntity {
    fn describe(self) -> String {
        "The counter entity holds {self.value} unit(s) of enterprise value"
    }
}
"#;

const FACTORY: &str = r#"import "../domain/counter_entity.vcwl"

export interface CounterServiceFactory {
    fn create(self, seed: Int) -> CounterEntity
}

export struct DefaultCounterServiceFactoryProvider {
    tag: String,
}

impl CounterServiceFactory for DefaultCounterServiceFactoryProvider {
    fn create(self, seed: Int) -> CounterEntity {
        CounterEntity { value: seed }
    }
}

export fn provide_counter_service_factory() -> DefaultCounterServiceFactoryProvider {
    DefaultCounterServiceFactoryProvider { tag: "default" }
}
"#;

const PRESENTATION: &str = r#"import "../domain/counter_entity.vcwl"
import "../application/counter_service_factory_provider.vcwl"

export interface CounterViewModel {
    fn headline(self) -> String
}

export struct DefaultCounterViewModel {
    entity: CounterEntity,
}

impl CounterViewModel for DefaultCounterViewModel {
    fn headline(self) -> String {
        self.entity.describe()
    }
}

export fn build_counter_view_model() -> DefaultCounterViewModel {
    let factory = provide_counter_service_factory()
    DefaultCounterViewModel { entity: factory.create(7) }
}

export component CounterView() {
    state clicks: Int = 0

    column(gap: 12px, align: center) {
        text(build_counter_view_model().headline())
        button("Escalate ({clicks})") {
            on click {
                clicks += 1
            }
        }
    }
}
"#;

const MAIN: &str = r#"app "{{name}}"

import "./domain/counter_entity.vcwl"
import "./application/counter_service_factory_provider.vcwl"
import "./presentation/counter_view.vcwl"

component App {
    style {
        background: #0b0d10
        color: #f5f5f5
        min-height: 100vh
        padding: 2rem
    }

    column(gap: 20px, align: center) {
        heading("{{name}} (Enterprise Edition)", level: 1)
        CounterView()
    }
}
"#;

pub fn new_enterprise(dir: &Path) -> Result<(), String> {
    if dir.exists()
        && dir
            .read_dir()
            .map(|mut d| d.next().is_some())
            .unwrap_or(true)
    {
        return Err(format!("{} already exists and is not empty", dir.display()));
    }
    let name = dir
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("app")
        .to_string();
    let files = [
        (
            "vcwl.toml",
            format!(
                "# VCWL project (enterprise)\nname = \"{name}\"\nentry = \"src/main.vcwl\"\nout = \"dist\"\n"
            ),
        ),
        ("src/main.vcwl", MAIN.replace("{{name}}", &name)),
        ("src/domain/counter_entity.vcwl", ENTITY.to_string()),
        (
            "src/application/counter_service_factory_provider.vcwl",
            FACTORY.to_string(),
        ),
        (
            "src/presentation/counter_view.vcwl",
            PRESENTATION.to_string(),
        ),
        (".gitignore", "dist/\n".to_string()),
    ];
    for (path, content) in files {
        let p = dir.join(path);
        std::fs::create_dir_all(p.parent().unwrap()).map_err(|e| e.to_string())?;
        std::fs::write(&p, content).map_err(|e| format!("{}: {e}", p.display()))?;
    }
    Ok(())
}
