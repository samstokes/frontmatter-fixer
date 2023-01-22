use std::fs::read_to_string;

use clap::Parser;
use eyre::{eyre, Context};
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

    /// Supply the files to fix as positional arguments
    paths: Vec<String>,
}

impl Config {
    fn script(&self) -> eyre::Result<String> {
        match (&self.inline_script, &self.script_path) {
            (Some(_), Some(_)) => Err(eyre!("can't specify both inline script and a script file")),
            (None, None) => Err(eyre!("must specify either inline script or a script file")),
            (Some(inline_script), _) => Ok(inline_script.clone()),
            (_, Some(script_path)) => read_to_string(&script_path)
                .context(format!("couldn't read script file {}", &script_path)),
        }
    }
}

fn main() -> eyre::Result<()> {
    let cfg = Config::parse();
    dbg!(&cfg);

    let processor = Processor::new(&cfg.script()?).context("couldn't create processor")?;

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
    script: String,
}

impl Processor {
    fn new(script: &str) -> eyre::Result<Self> {
        let lua = Lua::new();
        let fun = lua
            .load(script)
            .into_function()
            .context("lua script didn't compile")?;
        dbg!(fun);
        Ok(Self {
            lua,
            script: script.to_owned(),
        })
    }

    fn process(&self, path: &str) -> eyre::Result<()> {
        dbg!(path);

        let content = read_to_string(path).context("couldn't read file contents")?;
        dbg!(&content);

        let fixed_metadata = self.fix(&content)?;

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
        self.lua
            .load(&self.script)
            .exec()
            .context("error in Lua script")?;

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
        let processor = Processor::new(r#"meta.hello = meta.hello .. 'fish'"#)?;
        let fixed = processor.fix(EXAMPLE)?;
        assert_eq!("hello: worldfish\n", yaml::to_string(&fixed)?);
        Ok(())
    }

    #[test]
    #[allow(unused_must_use)]
    fn cant_fix() {
        let processor = Processor::new(r#""#).unwrap();
        processor
            .fix(EXAMPLE_NO_YFM)
            .expect_err("remove this test once this supports files with no frontmatter");
    }
}
