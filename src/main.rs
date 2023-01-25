use std::{fs::read_to_string, io};

use clap::Parser;
use eyre::{bail, eyre, Context};
use mlua::{Function, Lua, LuaSerdeExt, RegistryKey};
use serde_yaml as yaml;
use yaml_front_matter::YamlFrontMatter;

/// Run a Lua script to fix your frontmatter
#[derive(Debug, Parser)]
struct Config {
    /// Pass a short Lua script to run
    #[arg(short = 'e', long = "eval")]
    inline_script: Option<String>,
    /// Read a Lua script from a file
    #[arg(short = 'f', long = "script", id = "SCRIPT_FILE")]
    script_path: Option<String>,
    /// Run a Lua REPL
    #[arg(short = 'r', long = "repl")]
    repl: bool,
    /// Don't modify any files, just run script and show what would be done
    #[arg(short = 'n', long = "dry-run")]
    dry_run: bool,

    /// Supply the files to fix as positional arguments
    #[arg(id = "FILES")]
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

    let fixer = Fixer::new(cfg.script()?.as_deref()).context("couldn't setup")?;

    for path in cfg.paths {
        // TODO collect process errors
        process(&fixer, &path, cfg.dry_run).context(format!("couldn't process file {}", &path))?;
    }

    Ok(())
}

fn process(fixer: &Fixer, path: &str, dry_run: bool) -> eyre::Result<()> {
    dbg!(path);

    let content = read_to_string(path).context("couldn't read file contents")?;
    dbg!(&content);

    let (fixed_metadata, content) = fixer.fix(&content)?;

    if dry_run {
        println!("---");
        println!("{}", serde_yaml::to_string(&fixed_metadata)?);
        println!("---");
        println!("{}", content);
    } else {
        // TODO actually modify file instead of just printing frontmatter
        bail!("non-dry-run not yet implemented");
    }

    Ok(())
}

struct Fixer {
    lua: Lua,
    script: Option<RegistryKey>,
}

impl Fixer {
    fn new(script: Option<&str>) -> eyre::Result<Self> {
        let lua = Lua::new();

        let dump_fun = lua
            .create_function(lua_yaml_dump)
            .context("couldn't create yaml_dump function")?;
        lua.globals()
            .set("yaml_dump", dump_fun)
            .context("couldn't register yaml_dump function")?;

        let script_fun = script
            .map(|s| {
                lua.load(s)
                    .into_function()
                    .context("lua script didn't compile")
            })
            .transpose()?
            .map(|fun| {
                lua.create_registry_value(fun)
                    .expect("couldn't save precompiled script")
            });

        Ok(Self {
            lua,
            script: script_fun,
        })
    }

    fn fix(&self, content: &str) -> eyre::Result<(yaml::Value, String)> {
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
        globals
            .set("content", content.as_str())
            .context("couldn't send content to Lua")?;

        if let Some(script) = &self.script {
            let script_fun: Function = self
                .lua
                .registry_value(script)
                .expect("couldn't retrieve precompiled script");
            let _ = script_fun.call(()).context("error in Lua script")?;
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

        Ok((altered_metadata, content))
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
    # Title
    "#;

    const EXAMPLE_NO_YFM: &'_ str = "";

    #[test]
    fn empty_script_returns_frontmatter() -> eyre::Result<()> {
        let processor = Fixer::new(Some(""))?;
        let (yfm, _) = processor.fix(EXAMPLE)?;
        assert_eq!("hello: world\n", yaml::to_string(&yfm)?);
        Ok(())
    }

    #[test]
    fn passes_through_content() -> eyre::Result<()> {
        let processor = Fixer::new(Some(""))?;
        let (_, content) = processor.fix(EXAMPLE)?;
        assert_eq!("# Title", content.trim());
        Ok(())
    }

    #[test]
    fn script_can_access_and_modify_frontmatter() -> eyre::Result<()> {
        let processor = Fixer::new(Some(
            r#"
            meta.hello = meta.hello .. 'fish'
        "#,
        ))?;
        let (fixed, _) = processor.fix(EXAMPLE)?;
        assert_eq!("hello: worldfish\n", yaml::to_string(&fixed)?);
        Ok(())
    }

    #[test]
    fn script_can_access_content() -> eyre::Result<()> {
        let processor = Fixer::new(Some(
            r#"
            meta.hello = string.match(content, '# ([^%c]*)')
        "#,
        ))?;
        let (fixed, _) = processor.fix(EXAMPLE)?;
        assert_eq!("hello: Title\n", yaml::to_string(&fixed)?);
        Ok(())
    }

    #[test]
    fn script_cannot_modify_content() {
        let processor =
            Fixer::new(Some("content.fudge = 'vanilla'")).expect("script is valid, but...");
        let _ = processor
            .fix(EXAMPLE)
            .expect_err("content shouldn't be mutable");
    }

    #[test]
    fn script_cannot_replace_content() -> eyre::Result<()> {
        let processor = Fixer::new(Some("content = 'vanilla'"))?;
        let (_, content) = processor.fix(EXAMPLE)?;
        assert_eq!("# Title", content.trim());
        Ok(())
    }

    #[test]
    fn blows_up_if_no_frontmatter() {
        let processor = Fixer::new(Some(r#""#)).unwrap();
        let _ = processor
            .fix(EXAMPLE_NO_YFM)
            .expect_err("remove this test once this supports files with no frontmatter");
    }
}
