use std::{fs::read_to_string, io};

use clap::Parser;
use eyre::{bail, eyre, Context};
use mlua::{Lua, LuaSerdeExt};
use serde_yaml as yaml;
use yaml_front_matter::YamlFrontMatter;

/// Run a Lua script to fix your frontmatter
#[derive(Debug, Parser)]
struct Config {
    /// Pass a short Lua script to run
    #[arg(short = 'e', long = "eval")]
    inline_script: Option<String>,
    /// Read a Lua script from a file
    #[arg(short = 'f', long = "script")]
    script_path: Option<String>,
    /// Run a Lua REPL
    #[arg(short = 'r', long = "repl")]
    repl: bool,

    /// Supply the files to fix as positional arguments
    paths: Vec<String>,
}

impl Config {
    fn script(&self) -> eyre::Result<Option<String>> {
        match (&self.inline_script, &self.script_path, self.repl) {
            (Some(inline_script), None, false) => Ok(Some(inline_script.clone())),
            (None, Some(script_path), false) => read_to_string(&script_path)
                .context(format!("couldn't read script file {}", &script_path))
                .map(Some),
            (None, None, true) => Ok(None),
            (None, None, false) => Err(eyre!(
                "must specify one of inline script, a script file, or REPL"
            )),
            _ => Err(eyre!(
                "must specify only one of inline script, a script file, or REPL"
            )),
        }
    }
}

fn main() -> eyre::Result<()> {
    let cfg = Config::parse();
    dbg!(&cfg);

    let processor =
        Processor::new(cfg.script()?.as_deref()).context("couldn't create processor")?;

    for path in cfg.paths {
        // TODO collect process errors
        processor
            .process(&path)
            .context(format!("couldn't process file {}", &path))?;
    }

    Ok(())
}

struct Processor {
    lua: Lua,
    script: Option<String>,
}

impl Processor {
    fn new(script: Option<&str>) -> eyre::Result<Self> {
        let lua = Lua::new();

        if let Some(script) = script {
            let fun = lua
                .load(script)
                .into_function()
                .context("lua script didn't compile")?;
            dbg!(fun);
        }

        let dump_fun = lua
            .create_function(lua_yaml_dump)
            .context("couldn't create yaml_dump function")?;
        lua.globals()
            .set("yaml_dump", dump_fun)
            .context("couldn't register yaml_dump function")?;

        Ok(Self {
            lua,
            script: script.map(|s| s.to_owned()),
        })
    }

    fn process(&self, path: &str) -> eyre::Result<()> {
        dbg!(path);

        let content = read_to_string(path).context("couldn't read file contents")?;
        dbg!(&content);

        let fixed_metadata = self.fix(&content)?;

        // TODO actually modify file instead of just printing frontmatter
        println!("{}", serde_yaml::to_string(&fixed_metadata)?);

        Ok(())
    }

    fn fix(&self, content: &str) -> eyre::Result<yaml::Value> {
        // TODO handle files without frontmatter (stop using yaml_front_matter crate?)
        let yaml_front_matter::Document { metadata, content } =
            YamlFrontMatter::parse::<yaml::Value>(&content)
                .map_err(|e| eyre!("{}", e))
                .context("couldn't parse frontmatter")?;
        dbg!(&metadata);
        dbg!(&content);

        let globals = self.lua.globals();
        let lua_metadata = self
            .lua
            .to_value(&metadata)
            .context("couldn't convert metadata to Lua representation")?;
        globals
            .set("meta", lua_metadata)
            .context("couldn't send metadata to Lua")?;

        if let Some(script) = &self.script {
            self.lua
                .load(script)
                .exec()
                .context("error in Lua script")?;
        } else {
            let mut input = String::new();
            let stdin = io::stdin();
            while let Ok(len) = stdin.read_line(&mut input) {
                if len == 0 {
                    break;
                }
                match self.lua.load(&input).eval::<mlua::Value>() {
                    Ok(v) => println!("{:?}", v),
                    Err(e) => eprintln!("Error: {}", e),
                }
                input.clear();
            }
        }

        let altered_lua_metadata = globals
            .get("meta")
            .context("couldn't retrieve metadata from Lua")?;
        let altered_metadata: yaml::Value = self
            .lua
            .from_value(altered_lua_metadata)
            .context("couldn't convert metadata back from Lua representation")?;
        dbg!(&altered_metadata);
        Ok(altered_metadata)
    }
}

fn yaml_dump(v: &yaml::Value) -> eyre::Result<()> {
    let yaml = yaml::to_string(v)?;
    println!("{}", &yaml);
    Ok(())
}

fn lua_yaml_dump(lua: &Lua, v: mlua::Value) -> mlua::Result<()> {
    let yaml_v: yaml::Value = lua.from_value(v)?;
    yaml_dump(&yaml_v)
        .map_err(|e| mlua::Error::external(format!("couldn't format value as YAML: {:?}", e)))?;
    Ok(())
}

#[cfg(test)]
mod test {
    use super::*;

    const EXAMPLE: &'_ str = r#"
    ---
    hello: world
    ---
    "#;

    const EXAMPLE_NO_YFM: &'_ str = "";

    #[test]
    fn can_fix() -> eyre::Result<()> {
        let processor = Processor::new(Some(r#"meta.hello = meta.hello .. 'fish'"#))?;
        let fixed = processor.fix(EXAMPLE)?;
        assert_eq!("hello: worldfish\n", yaml::to_string(&fixed)?);
        Ok(())
    }

    #[test]
    #[allow(unused_must_use)]
    fn cant_fix() {
        let processor = Processor::new(Some(r#""#)).unwrap();
        processor
            .fix(EXAMPLE_NO_YFM)
            .expect_err("remove this test once this supports files with no frontmatter");
    }
}
