use std::{env::args, fs::read_to_string};

use eyre::{eyre, Context};
use mlua::{Lua, LuaSerdeExt};
use serde_yaml::Value as YValue;
use yaml_front_matter::YamlFrontMatter;

fn main() -> eyre::Result<()> {
    let paths = args().skip(1);

    let paths = paths.collect::<Vec<_>>();
    dbg!(&paths);

    for path in paths {
        // TODO collect process errors
        process(&path).context("couldn't process file")?;
    }

    Ok(())
}

fn process(path: &str) -> eyre::Result<()> {
    dbg!(path);

    let content = read_to_string(path).context("couldn't read file contents")?;
    dbg!(&content);

    // TODO handle files without frontmatter (stop using yaml_front_matter crate?)
    let yaml_front_matter::Document { metadata, content } =
        YamlFrontMatter::parse::<YValue>(&content)
            .map_err(|e| eyre!("{}", e))
            .context("couldn't parse frontmatter")?;
    dbg!(&metadata);
    dbg!(&content);

    // TODO reuse interpreter across paths
    let lua = Lua::new();
    let globals = lua.globals();
    let lua_metadata = lua
        .to_value(&metadata)
        .context("couldn't convert metadata to Lua representation")?;
    globals
        .set("meta", lua_metadata)
        .context("couldn't send metadata to Lua")?;
    lua.load(
        r#"
        print(meta.hello)
        meta.fish = 'bicycle'
        for i, tag in pairs(meta.tags) do
            local stripped_tag = string.gsub(tag, '%s*(%g*)%s*', '%1')
            print(stripped_tag)
            meta.tags[i] = stripped_tag
        end
        meta.tags = table.concat(meta.tags, ' ')
    "#,
    )
    .exec()
    .context("error in Lua script")?;

    let altered_lua_metadata = globals
        .get("meta")
        .context("couldn't retrieve metadata from Lua")?;
    let altered_metadata: YValue = lua
        .from_value(altered_lua_metadata)
        .context("couldn't convert metadata back from Lua representation")?;

    dbg!(&altered_metadata);
    println!("{}", serde_yaml::to_string(&altered_metadata)?);

    Ok(())
}
