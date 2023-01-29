mod frontmatter;

use std::{
    fs::read_to_string,
    io::{self, stdout},
};

use clap::Parser;
use eyre::{eyre, Context};
use mlua::{Function, Lua, LuaSerdeExt, RegistryKey};
use serde_yaml as yaml;
use tempfile::NamedTempFile;

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
    /// Print the name of each file being processed and its outcome
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,
    /// Print out the processed result of each file
    #[arg(short = 'p', long = "print")]
    print_result: bool,

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

    let fixer = Fixer::new(cfg.script()?.as_deref()).context("couldn't setup")?;

    let mut ok_paths: Vec<String> = Vec::new();
    let mut err_paths: Vec<(String, eyre::Report)> = Vec::new();

    let msg_process = if cfg.dry_run {
        "would process"
    } else {
        "processed"
    };
    let msg_fail = if cfg.dry_run {
        "would fail to process"
    } else {
        "failed to process"
    };

    for path in cfg.paths {
        match process(&fixer, &path, cfg.dry_run, cfg.print_result) {
            Ok(()) => {
                if cfg.verbose {
                    eprintln!("{} file {} successfully", msg_process, &path);
                }
                ok_paths.push(path);
            }
            Err(e) => {
                if cfg.verbose {
                    eprintln!("{} file {}: {:?}", msg_fail, &path, &e);
                }
                err_paths.push((path, e));
            }
        }
    }

    eprintln!(
        "{} {} files total",
        msg_process,
        ok_paths.len() + err_paths.len()
    );
    if !err_paths.is_empty() {
        eprintln!("{} {} files successfully", msg_process, ok_paths.len());
        eprintln!("{} {} files:", msg_fail, err_paths.len());
        for (path, err) in err_paths {
            eprintln!("{}: {:?}", path, err);
        }
    }

    Ok(())
}

fn process(fixer: &Fixer, path: &str, dry_run: bool, print_result: bool) -> eyre::Result<()> {
    let content = read_to_string(path).context("couldn't read file contents")?;

    let (fixed_metadata, content) = fixer.fix(&content)?;

    if print_result {
        frontmatter::write(stdout(), fixed_metadata.as_ref(), content)?;
    }
    if !dry_run {
        modify_file(path, fixed_metadata.as_ref(), content).context("couldn't modify file")?;
    }

    Ok(())
}

fn modify_file(path: &str, metadata: Option<&yaml::Value>, content: &str) -> eyre::Result<()> {
    let mut tmpfile = NamedTempFile::new()?;

    frontmatter::write(&mut tmpfile, metadata, content)
        .context("couldn't write fixed file to tempfile")?;
    tmpfile
        .persist(path)
        .context("couldn't rename tempfile over original path")?;
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

    fn fix<'this, 'doc>(
        &'this self,
        content: &'doc str,
    ) -> eyre::Result<(Option<yaml::Value>, &'doc str)> {
        let (metadata, content) = frontmatter::parse(content);

        let globals = self.lua.globals();
        if let Some(metadata) = metadata {
            let metadata = metadata.context("couldn't parse frontmatter")?;
            let lua_metadata = self
                .lua
                .to_value(&metadata)
                .context("couldn't convert metadata to Lua representation")?;
            globals
                .set("meta", lua_metadata)
                .context("couldn't send metadata to Lua")?;
        } else {
            // clear out previous file's meta
            globals
                .raw_remove("meta")
                .context("couldn't clear Lua metadata")?;
        }
        globals
            .set("content", content)
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
        let altered_metadata: Option<yaml::Value> = self
            .lua
            .from_value(altered_lua_metadata)
            .context("couldn't convert metadata back from Lua representation")?;

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

    const EXAMPLE: &'_ str = "\
    ---
    hello: world
    ---
    # Title
    ";

    const EXAMPLE_EMPTY_YFM: &'_ str = "\
    ---
    ---
    # Title
    ";

    const EXAMPLE_NO_YFM: &'_ str = "# Title\n";

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
    fn passes_through_content_if_no_frontmatter() -> eyre::Result<()> {
        let processor = Fixer::new(Some("")).unwrap();
        let (yfm, content) = processor.fix(EXAMPLE_NO_YFM)?;
        assert_eq!(None, yfm);
        assert_eq!("# Title", content.trim());
        Ok(())
    }

    #[test]
    fn blows_up_if_empty_frontmatter() {
        let processor = Fixer::new(Some("")).unwrap();
        let _ = processor
            .fix(EXAMPLE_EMPTY_YFM)
            .expect_err("malformed frontmatter should fail");
    }

    #[test]
    fn can_create_frontmatter_if_none() -> eyre::Result<()> {
        let processor = Fixer::new(Some("meta = { hello = 'world' }")).unwrap();
        let (yfm, _) = processor.fix(EXAMPLE_NO_YFM)?;
        assert_eq!("hello: world\n", yaml::to_string(&yfm)?);
        Ok(())
    }
}
