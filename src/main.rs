use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand, ValueEnum};
use comfy_table::Table;
use dom_smoothie::{Article, Readability};
use jiff::{Timestamp, Unit, Zoned};
use rusqlite::{Connection, Transaction};
use serde::Deserialize;
use std::path::PathBuf;
use ureq::Agent;
use url::Url;
use uuid::Uuid;

mod db_migrations;

// Table IDs are v7 UUIDs, handled via sqlite3 BLOB; this means that we can potentially
// merge two databases without stepping on foreign entries.
type TableId = Uuid;

static APP_NAME: &str = env!("CARGO_PKG_NAME");
static APP_USER_AGENT: &str = concat!(env!("CARGO_PKG_NAME"), "/", env!("CARGO_PKG_VERSION"),);

#[derive(Clone, Debug, Default, ValueEnum)]
enum ListOutputFormat {
    #[default]
    Table,
}

// NB See https://rust-cli-recommendations.sunshowers.io/handling-arguments.html
// for advice on structuring the subcommands
#[derive(Debug, Parser)]
#[command(version, about, long_about = None)]
#[command(propagate_version = true)]
struct Cli {
    #[command(subcommand)]
    command: Commands,

    /// Path to the config file to use
    #[clap(long, global = true)]
    config: Option<PathBuf>,
    /// Path to the database to use
    #[clap(long, global = true)]
    db: Option<PathBuf>,
}

#[derive(Debug, Deserialize)]
struct Config {
    #[serde(default)]
    database: PathBuf,
}

impl Config {
    fn new() -> Self {
        Default::default()
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            database: default_db_location(),
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Link {
    id: TableId,
    url: Url,
    title: Option<String>,
    description: Option<String>,
    content: Option<String>,
    is_primary: bool,
    created_at: Timestamp,
    modified_at: Timestamp,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Note {
    id: TableId,
    content: String,
    title: String,
    link_id: Option<TableId>,
    created_at: Timestamp,
    modified_at: Timestamp,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
struct Tag {
    id: TableId,
    name: String,
    slug: String,
    created_at: Timestamp,
    modified_at: Timestamp,
}

#[derive(Parser, Debug, Default)]
struct AddArgs {
    /// The URL to add
    link: String,
    /// Tag for the link; multiple are allowed
    #[arg(short, long, num_args = 1..)]
    tag: Vec<String>,
    /// User-provided description for the link
    #[arg(long)]
    description: Option<String>,
    /// User-provided title for the link
    #[arg(long)]
    title: Option<String>,
    /// Add a freeform note for this link
    #[arg(short, long, action)]
    note: bool,
    /// Add a short note directly from the command line
    #[arg(short, long, conflicts_with = "note")]
    message: Option<String>,
    /// An optional related link (such as discussion of the primary link, or the
    /// site where the link was found)
    #[arg(long)]
    related_link: Option<String>,
    /// Optional context for the related link (e.g. "via" or "lobsters")
    #[arg(long, requires = "related_link")]
    relation: Option<String>,
}

#[derive(Parser, Debug, Default)]
struct ListArgs {
    /// Format of the output
    #[arg(long, value_enum, default_value_t=ListOutputFormat::Table)]
    format: ListOutputFormat,
    /// Show only links matching one or more tags
    #[arg(short, long, num_args = 1..)]
    tag: Vec<String>,
}

#[derive(Parser, Debug, Default)]
struct NoteArgs {
    /// Tag for the note; multiple are allowed
    #[arg(short, long, num_args = 1..)]
    tag: Vec<String>,
    /// Title for the note
    #[arg(long)]
    title: Option<String>,
    /// Add a short note directly from the command line
    #[arg(short, long)]
    message: Option<String>,
}

#[derive(Parser, Debug, Default)]
struct RemoveArgs {
    /// The note or link to remove
    item: String,
}

#[derive(Parser, Debug, Default)]
struct SearchArgs {
    /// The term to search
    term: String,
    /// Format of the output
    #[arg(long, value_enum, default_value_t=ListOutputFormat::Table)]
    format: ListOutputFormat,
}

#[derive(Parser, Debug, Default)]
struct ShowArgs {
    /// The link or note to display in detail
    term: String,
    /// Format of the output
    #[arg(long, value_enum, default_value_t=ListOutputFormat::Table)]
    format: ListOutputFormat,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Add a link
    Add {
        #[clap(flatten)]
        add_args: AddArgs,
    },
    /// Show all links
    #[clap(alias = "ls")]
    List {
        #[clap(flatten)]
        list_args: ListArgs,
    },
    /// Add a freeform note
    Note {
        #[clap(flatten)]
        note_args: NoteArgs,
    },
    /// Remove a link or note
    #[clap(alias = "rm")]
    Remove {
        #[clap(flatten)]
        remove_args: RemoveArgs,
    },
    /// Full-text search of link contents
    Search {
        #[clap(flatten)]
        search_args: SearchArgs,
    },
    /// Show link details
    Show {
        #[clap(flatten)]
        show_args: ShowArgs,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = load_config(&cli)?;
    if let Some(parent) = config.database.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "Unable to create database at {}",
                config.database.to_string_lossy()
            )
        })?;
    }
    let conn = Connection::open(&config.database)
        .with_context(|| format!("Unable to open database at {:?}", &config.database))?;
    db_migrations::migrate(conn)
        .with_context(|| format!("Unable to upgrade database at {:?}", &config.database))?;

    match &cli.command {
        Commands::Add { add_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            add_cmd(&tx, add_args).with_context(|| format!("Unable to add <{}>", add_args.link))?;
            tx.commit()?;
        }
        Commands::List { list_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            list_cmd(&tx, list_args).with_context(|| "Unable to list items")?;
        }
        Commands::Note { note_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            note_cmd(&tx, note_args).with_context(|| "Unable to add note")?;
            tx.commit()?;
        }
        Commands::Remove { remove_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            remove_cmd(&tx, remove_args).with_context(|| "Unable to remove item")?;
            tx.commit()?;
        }
        Commands::Search { search_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            search_cmd(&tx, search_args).with_context(|| "Unable to search")?;
        }
        Commands::Show { show_args } => {
            let mut conn = Connection::open(&config.database)?;
            let tx = conn.transaction()?;
            show_cmd(&tx, show_args)
                .with_context(|| format!("Unable to show <{}>", show_args.term))?;
        }
    }
    Ok(())
}

fn home_dir() -> Option<PathBuf> {
    // NB: The state of std::env::home_dir() and its replacements is a mess.
    // See <https://doc.rust-lang.org/std/env/fn.home_dir.html> and
    // <https://github.com/rust-lang/libs-team/issues/372>. Notably, `home`
    // is not recommended for use outside of Cargo. Hopefully `env_home` will
    // end up in standard library and we can go ahead and use that.
    env_home::env_home_dir()
}

fn expand_tilde(path: &mut PathBuf) {
    let home = home_dir();
    if let Some(home) = home {
        let mut rewritten = PathBuf::new();
        rewritten.push(home);
        for arg in path.iter().skip(1) {
            rewritten.push(arg);
        }
        *path = rewritten;
    }
}

fn default_db_location() -> PathBuf {
    let app_dirs = platform_dirs::AppDirs::new(Some(APP_NAME), true);
    match app_dirs {
        Some(app_dirs) => app_dirs.data_dir.join("meowpad.db"),
        None => match home_dir() {
            Some(mut home_dir) => {
                home_dir.push(".meowpad.db");
                home_dir
            }
            None => ".meowpad.db".into(),
        },
    }
}

fn load_config(cli: &Cli) -> Result<Config> {
    // Defaults will be overwritten by the TOML config file, which in turn will
    // be overwritten by CLI arguments, if available.
    let mut config = Config::new();
    let mut error_on_load_failure = false;
    let config_path = if let Some(cli_config) = &cli.config {
        error_on_load_failure = true;
        expand_tilde(&mut cli_config.clone());
        cli_config
    } else {
        // It may make sense at some point to switch from `platform_dirs` to
        // `etcetera` or `xdg` to reduce the number of dependencies that get
        // pulled in. We're using `platform_dirs` for now because it handles
        // Windows (less important) and lets us specify that Macs should
        // follow XDG locations (important).
        let app_dirs = platform_dirs::AppDirs::new(Some(APP_NAME), true);
        match app_dirs {
            Some(app_dirs) => &app_dirs.config_dir.join("config.toml"),
            // This will error out, which is fine!
            None => &PathBuf::new(),
        }
    };
    if let Ok(config_str) = std::fs::read_to_string(config_path) {
        config = toml::from_str(&config_str).with_context(|| {
            format!(
                "Unable to parse config file at {}",
                config_path.to_string_lossy()
            )
        })?;
    } else {
        // If we are just using a default config path and there is no config present,
        // we'll treat it as a noop and stick with the default config.
        if error_on_load_failure {
            return Err(anyhow!(
                "Unable to open config file at {}",
                config_path.to_string_lossy()
            ));
        }
    }
    // If we ever want to support setting options via ENV variables,
    // they would go here. Then, any values that can be overwritten
    // from the CLI should go last.
    if let Some(cli_db) = &cli.db {
        config.database = cli_db.to_path_buf();
    }
    // Finally, let's do tilde expansion on file paths if needed.
    if config.database.starts_with("~/") {
        expand_tilde(&mut config.database);
    }
    Ok(config)
}

// UTIL
fn now() -> Result<String> {
    let zoned = Zoned::now().round(Unit::Second)?;
    Ok(zoned.timestamp().to_string())
}

// LINK
fn readability(url: &str) -> Result<Article> {
    let agent: Agent = Agent::config_builder()
        .user_agent(APP_USER_AGENT)
        .timeout_global(Some(std::time::Duration::from_secs(5)))
        .build()
        .into();
    let html: String = agent.get(url).call()?.body_mut().read_to_string()?;
    // TODO: We should test to see if we believe that the readability score is
    // high enough to make this worthwhile, or if we should instead just
    // extract the title (and maybe excerpt?).
    let mut readability = Readability::new(html, Some(url), None)?;
    Ok(readability.parse()?)
}

// UTIL
fn get_tag_id(tx: &Transaction, tag_name: &str) -> Result<TableId> {
    let now = now()?;
    let slug = util::slugify(tag_name)?;
    let id = db::require_tag(tx, tag_name, &slug, &now)?;
    Ok(id)
}

fn add_cmd(tx: &Transaction, args: &AddArgs) -> Result<()> {
    let url =
        Url::parse(&args.link).with_context(|| format!("{} is an invalid URL", &args.link))?;
    let scheme = url.scheme();
    if scheme != "https" && scheme != "http" {
        return Err(anyhow!("Non-web URL scheme {}", scheme));
    }
    let now = now()?;
    // TODO: We should be able to disable fetch via the command-line, everywhere
    // via config, or on a per-domain or per-tag basis.
    let page_info = readability(args.link.as_ref())?;
    let title = if args.title.is_some() {
        args.title.as_deref()
    } else if page_info.title.is_empty() {
        None
    } else {
        Some(page_info.title.as_ref())
    };
    let description = if args.description.is_some() {
        args.description.as_deref()
    } else {
        page_info.excerpt.as_deref()
    };
    let text_content = page_info.text_content.trim();

    let link_insert_args = db::LinkInsert {
        url: args.link.as_ref(),
        title,
        description,
        content: Some(text_content),
        is_primary: true,
        timestamp: &now,
    };

    let link_result = db::insert_link(tx, &link_insert_args, false);

    let link_id = if let Ok(new_link) = link_result {
        new_link
    } else {
        // Let's see if we have an existing *secondary* link that we are changing
        // to a primary (so it can have its own tags, notes, etc.)
        let mut secondary_link = db::get_link(
            tx,
            db::TermOrId::Term(args.link.as_ref()),
            db::IsPrimary::SecondaryOnly,
        )?;
        if let Some(ref mut secondary_link) = secondary_link {
            secondary_link.title = link_insert_args.title.map(|s| s.to_string());
            secondary_link.description = link_insert_args.description.map(|s| s.to_string());
            secondary_link.is_primary = true;
            db::update_link(tx, secondary_link)?;
            // A secondary link should never have attached content.
            db::insert_content(tx, &secondary_link.id, text_content)?;
        } else {
            anyhow::bail!("Unable to insert <{}>; is it a duplicate?", args.link);
        };
        secondary_link.unwrap().id
    };

    for tag_name in &args.tag {
        let tag_id = get_tag_id(tx, tag_name)?;
        db::tag_link(tx, link_id, tag_id)?;
    }

    // NB: We don't currently need to do any kind of checking on note existence
    // or updating a note, because we don't currently allow link editing/--force,
    // but when that changes, this should chage as well.
    let note = if let Some(message) = &args.message {
        Some(message.clone())
    } else if args.note {
        Some(edit::edit("")?)
    } else {
        None
    };

    if let Some(note_text) = note {
        let note_id = db::upsert_note(tx, &note_text, &args.link, Some(&link_id), &now)?;
        for tag_name in &args.tag {
            let tag_id = get_tag_id(tx, tag_name)?;
            db::tag_note(tx, note_id, tag_id)?;
        }
    }

    if let Some(related_link) = &args.related_link {
        // TODO: We should I think grab title using Readability, even if we don't
        // need or want description or contents.
        let insert_vals = db::LinkInsert {
            url: related_link,
            title: None,
            description: None,
            content: None,
            is_primary: false,
            timestamp: &now,
        };
        let related_link_id = db::insert_link(tx, &insert_vals, true)?;
        db::relate_links(tx, link_id, related_link_id, args.relation.as_deref())?;
    }

    println!("Added bookmark for <{}>", args.link);
    Ok(())
}

fn list_cmd(tx: &Transaction, args: &ListArgs) -> Result<()> {
    let tags = if args.tag.is_empty() {
        vec![]
    } else {
        args.tag
            .iter()
            .map(|t| util::slugify(t))
            .collect::<Result<Vec<_>>>()?
    };
    let items = db::get_links(tx, tags, None)?;
    let output = match args.format {
        ListOutputFormat::Table => list_as_table(items)?,
    };
    println!("{output}");
    Ok(())
}

fn link_as_table(
    link: Link,
    tags: Vec<Tag>,
    note: Option<Note>,
    related_links: Vec<(String, Option<String>)>,
) -> Result<String> {
    let mut table = Table::new();
    table
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
        .load_preset(comfy_table::presets::UTF8_FULL)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS);
    table.add_row(vec![
        "Title",
        link.title.as_ref().unwrap_or(&"".to_string()),
    ]);
    table.add_row(vec!["URL", link.url.as_ref()]);
    table.add_row(vec![
        "Description",
        link.description.as_ref().unwrap_or(&"".to_string()),
    ]);
    table.add_row(vec![
        "Added".to_string(),
        link.created_at.strftime("%F").to_string(),
    ]);
    if !tags.is_empty() {
        table.add_row(vec![
            "Tags".to_string(),
            tags.iter()
                .map(|t| t.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ]);
    }
    if !related_links.is_empty() {
        table.add_row(vec![
            "See Also".to_string(),
            related_links
                .iter()
                .map(|rl| {
                    if let Some(relation) = &rl.1 {
                        format!("{} ({relation})", rl.0)
                    } else {
                        rl.0.to_string()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ]);
    }
    if let Some(note) = note {
        let content = note.content.as_str().trim();
        table.add_row(vec!["Note", content]);
    }
    Ok(table.to_string())
}

fn list_as_table(items: Vec<Link>) -> Result<String> {
    let mut table = Table::new();
    table
        .set_header(vec!["URL", "Title", "Created"])
        .set_content_arrangement(comfy_table::ContentArrangement::Dynamic)
        .load_preset(comfy_table::presets::UTF8_BORDERS_ONLY)
        .apply_modifier(comfy_table::modifiers::UTF8_ROUND_CORNERS);
    for item in &items {
        table.add_row(vec![
            &item.url.to_string(),
            item.title.as_ref().unwrap_or(&"".to_string()),
            &item.created_at.strftime("%F").to_string(),
        ]);
    }
    Ok(table.to_string())
}

fn note_cmd(tx: &Transaction, args: &NoteArgs) -> Result<()> {
    let now = now()?;
    let title = match &args.title {
        Some(given_title) => given_title,
        None => &now,
    };
    let content = match db::get_note_by_title(tx, title)? {
        Some(existing_note) => existing_note.content,
        None => "".to_string(),
    };
    let note = if let Some(message) = &args.message {
        if content.is_empty() {
            message.clone()
        } else {
            let mut new_note = content;
            new_note.push('\n');
            new_note.push_str(message);
            new_note
        }
    } else {
        edit::edit(content)?
    };
    if note.is_empty() {
        println!("No note to add");
    } else {
        let note_id = db::upsert_note(tx, &note, title, None, &now)?;
        for tag_name in &args.tag {
            let tag_id = get_tag_id(tx, tag_name)?;
            db::tag_note(tx, note_id, tag_id)?;
        }
        println!("Added note <{}>", &title);
    }
    Ok(())
}

fn remove_cmd(tx: &Transaction, args: &RemoveArgs) -> Result<()> {
    let item = &args.item;
    let mut which: Vec<&str> = vec![];
    if let Some(mut link) = db::get_link(tx, db::TermOrId::Term(item), db::IsPrimary::PrimaryOnly)?
    {
        let inverse_relations = db::get_inverse_related_links(tx, &link.id)?;
        if inverse_relations.is_empty() {
            db::delete_link(tx, &link.id)?;
        } else {
            link.is_primary = false;
            db::update_link(tx, &link)?;
            db::delete_item_tags(tx, &link.id)?;
            db::delete_related_links(tx, Some(&link.id), None)?;
            db::delete_content(tx, &link.id)?;
        }
        which.push("link");
    }
    if let Some(note) = db::get_note_by_title(tx, item)? {
        db::delete_note(tx, &note.id)?;
        which.push("note");
    }
    if which.is_empty() {
        println!("<{item}> not found");
    } else {
        let message = which.join(" and ");
        println!("Removed {message} for <{item}>");
    }
    Ok(())
}

fn search_cmd(tx: &Transaction, args: &SearchArgs) -> Result<()> {
    let search_term = &args.term;
    let link_items = db::search_links(tx, search_term.as_str())?;
    let output = match args.format {
        ListOutputFormat::Table => list_as_table(link_items)?,
    };
    println!("{output}");
    Ok(())
}

fn show_cmd(tx: &Transaction, args: &ShowArgs) -> Result<()> {
    let link = db::get_link(
        tx,
        db::TermOrId::Term(args.term.as_str()),
        db::IsPrimary::PrimaryOnly,
    )?;
    let output = if let Some(link) = link {
        let tags = db::tags_for_item(tx, &link.id)?;
        let note = db::get_note_by_link_id(tx, &link.id)?;
        let related_links = db::related_links(tx, &link.id)?;
        match args.format {
            ListOutputFormat::Table => link_as_table(link, tags, note, related_links)?,
        }
    } else {
        format!("<{}> not found", args.term).to_string()
    };
    println!("{output}");
    Ok(())
}

mod db {
    use anyhow::{anyhow, Result};
    use rusqlite::{named_params, params_from_iter, ToSql, Transaction};
    use uuid::Uuid;

    type TableId = super::TableId;

    #[allow(dead_code)]
    pub enum IsPrimary {
        PrimaryOnly,
        SecondaryOnly,
        Either,
    }

    pub enum TermOrId<'a> {
        Term(&'a str),
        Id(TableId),
    }

    impl ToSql for TermOrId<'_> {
        fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
            match *self {
                TermOrId::Term(term) => Ok(term.into()),
                TermOrId::Id(id) => Ok(id.into()),
            }
        }
    }

    fn get_uuid() -> Uuid {
        let now = jiff::Timestamp::now();
        // NB: We should grab the microseconds and use them instead of 0.
        let ts = uuid::Timestamp::from_unix(uuid::NoContext, now.as_second() as u64, 0);
        Uuid::new_v7(ts)
    }

    // LINKS
    pub fn get_links(
        tx: &Transaction,
        tags: Vec<String>,
        search_term: Option<&str>,
    ) -> Result<Vec<super::Link>> {
        let select = "SELECT
            id, url, title, description, is_primary, created_at, modified_at
            FROM link
            ";
        let where_clause = "WHERE is_primary IS TRUE";
        let tag_filter = if tags.is_empty() {
            "".to_string()
        } else {
            let qmarks: Vec<&str> = tags.iter().map(|_| "?").collect();
            let joined = qmarks.join(", ");
            format!(
                "AND id in (SELECT link_id FROM item_tag WHERE tag_id in
            (SELECT id FROM tag WHERE slug IN ({joined})))"
            )
        };
        let search_filter = if search_term.is_some() {
            "AND id in (SELECT link_id FROM link_content
            WHERE link_content MATCH ?)"
                .to_string()
        } else {
            "".to_string()
        };
        let order = "ORDER BY created_at DESC";
        let query = format!(
            "{} {} {} {} {}",
            select, where_clause, tag_filter, search_filter, order
        );
        let mut stmt = tx.prepare(query.as_ref())?;
        let mut all_params = tags;
        if let Some(term) = search_term {
            all_params.push(term.to_string());
        }
        let query_params = params_from_iter(all_params.iter());
        let mut rows = stmt.query(query_params)?;
        let mut resp: Vec<super::Link> = vec![];
        while let Some(row) = rows.next()? {
            resp.push(super::Link {
                id: row.get(0)?,
                url: row.get(1)?,
                title: Some(row.get::<_, String>(2)?),
                description: Some(row.get::<_, String>(3)?),
                // In the context of a bulk get, we don't need to fetch the
                // content value at this time.
                content: None,
                is_primary: row.get(4)?,
                created_at: row.get::<_, String>(5)?.parse()?,
                modified_at: row.get::<_, String>(6)?.parse()?,
            })
        }
        Ok(resp)
    }

    pub fn get_link(
        tx: &Transaction,
        identifier: TermOrId,
        is_primary: IsPrimary,
    ) -> Result<Option<super::Link>> {
        let insert = "SELECT
            id, url, title, description, is_primary, created_at, modified_at
            FROM link
            ";
        let where_clause = match is_primary {
            IsPrimary::PrimaryOnly => "WHERE is_primary IS TRUE",
            IsPrimary::SecondaryOnly => "WHERE is_primary IS FALSE",
            _ => "WHERE 1 = 1",
        };
        let id_filter = match identifier {
            TermOrId::Term(_) => "AND url = ?",
            TermOrId::Id(_) => "AND id = ?",
        };
        let query = format!("{} {} {}", insert, where_clause, id_filter);
        let mut stmt = tx.prepare(query.as_ref())?;
        let mut rows = stmt.query([identifier])?;
        if let Some(row) = rows.next()? {
            let mut link = super::Link {
                id: row.get(0)?,
                url: row.get(1)?,
                title: row.get::<_, Option<String>>(2)?,
                description: row.get::<_, Option<String>>(3)?,
                content: None,
                is_primary: row.get(4)?,
                created_at: row.get::<_, String>(5)?.parse()?,
                modified_at: row.get::<_, String>(6)?.parse()?,
            };
            let mut stmt =
                tx.prepare("SELECT content FROM link_content WHERE link_id = ?".as_ref())?;
            let mut content_rows = stmt.query([link.id.to_string()])?;
            if let Some(row) = content_rows.next()? {
                link.content = row.get(0)?;
            };
            Ok(Some(link))
        } else {
            Ok(None)
        }
    }

    pub fn delete_link(tx: &Transaction, link_id: &TableId) -> Result<()> {
        // If the link is a primary link but also serves as a related link,
        // we want to make it is_primary FALSE and also drop related links,
        // associated notes, and tags; in the normal course of things, however,
        // our foreign key cascades will clean them up.
        //
        // A possible improvement would be to check here and remove orphaned
        // tags.
        let delete_query = "DELETE FROM link WHERE id = ? AND is_primary = true";
        tx.execute(delete_query, [&link_id])?;
        Ok(())
    }

    pub fn get_inverse_related_links(tx: &Transaction, link_id: &TableId) -> Result<Vec<TableId>> {
        let select = "SELECT primary_link_id
                FROM related_link
                WHERE related_link_id = ?";
        let mut stmt = tx.prepare(select)?;
        let mut rows = stmt.query([link_id])?;
        let mut resp: Vec<TableId> = vec![];
        while let Some(row) = rows.next()? {
            resp.push(row.get(0)?);
        }
        Ok(resp)
    }

    #[derive(Debug)]
    pub struct LinkInsert<'a> {
        pub url: &'a str,
        pub title: Option<&'a str>,
        pub description: Option<&'a str>,
        pub content: Option<&'a str>,
        pub is_primary: bool,
        pub timestamp: &'a str,
    }

    pub fn insert_link(
        tx: &Transaction,
        link: &LinkInsert,
        ignore_conflict: bool,
    ) -> Result<TableId> {
        let id = get_uuid();
        let values = named_params! {
            ":id": id,
            ":url": link.url,
            ":title": link.title,
            ":description": link.description,
            ":is_primary": link.is_primary,
            ":created_at": link.timestamp,
            ":modified_at": link.timestamp,
        };
        let insert = "INSERT INTO link
            (id, url, title, description, is_primary, created_at, modified_at)
            VALUES(:id, :url, :title, :description, :is_primary, :created_at, :modified_at)
            ";
        // We can't simply "DO NOTHING", because that terminates the query
        // and we don't return an id; instead we'll update something that
        // should be the same.
        let conflict = if ignore_conflict {
            "ON CONFLICT DO UPDATE
            SET url = :url
            "
        } else {
            ""
        };
        let returning = "RETURNING id";
        let query = format!("{} {} {}", insert, conflict, returning);
        let mut stmt = tx.prepare(query.as_ref())?;
        let mut rows = stmt.query(values)?;
        let row_result = if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Err(anyhow!("Unable to insert link `{}`", link.url))
        };
        // Now, insert the content into the full-text index.
        if let Ok(row_id) = row_result {
            if let Some(content) = link.content {
                insert_content(tx, &row_id, content)?;
            }
        }
        row_result
    }

    pub fn update_link(tx: &Transaction, link: &super::Link) -> Result<Option<super::Link>> {
        let values = named_params! {
            ":id": link.id,
            ":url": link.url,
            ":title": link.title,
            ":description": link.description,
            ":is_primary": link.is_primary,
            ":modified_at": super::now()?,
        };
        let query = "UPDATE link SET
            url = :url,
            title = :title,
            description = :description,
            is_primary = :is_primary,
            modified_at = :modified_at
            WHERE id = :id";
        let mut stmt = tx.prepare(query)?;
        stmt.execute(values)?;
        get_link(tx, TermOrId::Id(link.id), IsPrimary::Either)
    }

    pub fn tag_link(tx: &Transaction, link_id: TableId, tag_id: TableId) -> Result<()> {
        let query = "INSERT INTO item_tag (link_id, tag_id)
        VALUES (?1, ?2)
        ON CONFLICT DO NOTHING";
        tx.execute(query, [&link_id, &tag_id])?;
        Ok(())
    }

    pub fn tag_note(tx: &Transaction, note_id: TableId, tag_id: TableId) -> Result<()> {
        let query = "INSERT INTO item_tag (note_id, tag_id)
        VALUES (?1, ?2)
        ON CONFLICT DO NOTHING";
        tx.execute(query, [&note_id, &tag_id])?;
        Ok(())
    }

    pub fn relate_links(
        tx: &Transaction,
        primary_id: TableId,
        secondary_id: TableId,
        relationship: Option<&str>,
    ) -> Result<()> {
        let related_values = named_params! {
            ":primary_id": primary_id,
            ":secondary_id": secondary_id,
            ":relationship": relationship,
        };
        tx.execute(
            "INSERT INTO related_link
                (primary_link_id, related_link_id, relationship)
                VALUES(:primary_id, :secondary_id, :relationship)",
            related_values,
        )?;
        Ok(())
    }

    pub fn related_links(
        tx: &Transaction,
        primary_id: &TableId,
    ) -> Result<Vec<(String, Option<String>)>> {
        let query = "SELECT
            url, related_link.relationship
            FROM link JOIN related_link
            ON link.id = related_link.primary_link_id
            WHERE id = ?
            ";
        let mut stmt = tx.prepare(query)?;
        let mut rows = stmt.query([&primary_id])?;
        let mut resp: Vec<(String, Option<String>)> = vec![];
        while let Some(row) = rows.next()? {
            resp.push((row.get(0)?, row.get(1)?));
        }
        Ok(resp)
    }

    pub fn delete_related_links(
        tx: &Transaction,
        primary_link_id: Option<&TableId>,
        related_link_id: Option<&TableId>,
    ) -> Result<()> {
        if primary_link_id.is_none() && related_link_id.is_none() {
            Err(anyhow!("Primary or related link ID required"))
        } else {
            let query_base = "DELETE FROM related_link
                WHERE";
            let mut query_vec = vec![];
            let mut vals_vec = vec![];
            if let Some(primary_link) = primary_link_id {
                query_vec.push("\t\t\tprimary_link_id = ?");
                vals_vec.push(primary_link);
            }
            if let Some(related_link) = related_link_id {
                query_vec.push("\t\t\trelated_link_id = ?");
                vals_vec.push(related_link);
            }
            let query = format!("{} {}", query_base, query_vec.join("\t\t\tAND"));
            let mut stmt = tx.prepare(&query)?;
            stmt.execute(params_from_iter(vals_vec.iter()))?;
            Ok(())
        }
    }

    pub fn insert_content(tx: &Transaction, link_id: &TableId, content: &str) -> Result<()> {
        let ft_query = "INSERT INTO link_content(link_id, content)
            VALUES (:id, :content)";
        let mut ft_stmt = tx.prepare(ft_query)?;
        let ft_values = named_params! {
            ":id": link_id,
            ":content": content,
        };
        ft_stmt.execute(ft_values)?;
        Ok(())
    }

    pub fn delete_content(tx: &Transaction, link_id: &TableId) -> Result<()> {
        let ft_query = "DELETE FROM link_content
            WHERE link_id = :id";
        let mut ft_stmt = tx.prepare(ft_query)?;
        let ft_values = named_params! {
            ":id": link_id,
        };
        ft_stmt.execute(ft_values)?;
        Ok(())
    }

    // TAGS
    pub fn tags_for_item(tx: &Transaction, item_id: &TableId) -> Result<Vec<super::Tag>> {
        let query = "SELECT DISTINCT id, slug, name, created_at, modified_at
            FROM tag
            WHERE id IN (
                SELECT tag_id FROM item_tag
                WHERE note_id = ?1 OR link_id = ?2
            ) ORDER BY slug";
        let mut stmt = tx.prepare(query)?;
        let mut rows = stmt.query([&item_id, &item_id])?;
        let mut tags: Vec<super::Tag> = vec![];
        while let Some(row) = rows.next()? {
            tags.push(super::Tag {
                id: row.get(0)?,
                slug: row.get(1)?,
                name: row.get(2)?,
                created_at: row.get::<_, String>(3)?.parse()?,
                modified_at: row.get::<_, String>(4)?.parse()?,
            });
        }
        Ok(tags)
    }

    pub fn require_tag(
        tx: &Transaction,
        name: &str,
        slug: &str,
        timestamp: &str,
    ) -> Result<TableId> {
        let id = get_uuid();
        let query = "INSERT INTO tag (id, slug, name, created_at, modified_at)
        VALUES (:id, :slug, :name, :created_at, :modified_at)
        ON CONFLICT DO UPDATE
        SET slug = :slug
        RETURNING id";
        let mut stmt = tx.prepare(query)?;
        let values = named_params! {
            ":id": id,
            ":slug": slug,
            ":name": name,
            ":created_at": timestamp,
            ":modified_at": timestamp,
        };
        let mut rows = stmt.query(values)?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Err(anyhow!("Unable to insert or load tag `{}`", slug))
        }
    }

    pub fn delete_item_tags(tx: &Transaction, item_id: &TableId) -> Result<()> {
        let query = "DELETE FROM item_tag WHERE note_id = ?1 OR link_id = ?2";
        let mut stmt = tx.prepare(query)?;
        stmt.execute([&item_id, &item_id])?;
        Ok(())
    }

    // NOTES
    pub fn upsert_note(
        tx: &Transaction,
        note: &str,
        title: &str,
        link_id: Option<&TableId>,
        timestamp: &str,
    ) -> Result<TableId> {
        let id = get_uuid();
        let values = named_params! {
            ":id": id,
            ":content": note,
            ":title": title,
            ":link_id": link_id,
            ":created_at": timestamp,
            ":modified_at": timestamp,
        };
        // We can't simply "DO NOTHING" for the reasons given in insert_link().
        let query = "INSERT INTO note
            (id, content, title, link_id, created_at, modified_at)
            VALUES(:id, :content, :title, :link_id, :created_at, :modified_at)
            ON CONFLICT(title) DO UPDATE
            SET content = :content, modified_at = :modified_at
            WHERE title = :title
            RETURNING id";
        let mut stmt = tx.prepare(query)?;
        let mut rows = stmt.query(values)?;
        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Err(anyhow!("Unable to insert note `{}`", title))
        }
    }

    pub fn get_note_by_title(tx: &Transaction, title: &str) -> Result<Option<super::Note>> {
        get_note(tx, None, None, Some(title))
    }

    pub fn get_note_by_link_id(tx: &Transaction, link_id: &TableId) -> Result<Option<super::Note>> {
        get_note(tx, None, Some(link_id), None)
    }

    fn get_note(
        tx: &Transaction,
        id: Option<&TableId>,
        link_id: Option<&TableId>,
        title: Option<&str>,
    ) -> Result<Option<super::Note>> {
        let mut filters: Vec<&str> = vec![];
        let mut values: Vec<Box<dyn ToSql>> = vec![];
        if let Some(id) = id {
            filters.push("id = ?");
            values.push(Box::new(id));
        }
        if let Some(link_id) = link_id {
            filters.push("link_id = ?");
            values.push(Box::new(link_id));
        }
        if let Some(title) = title {
            filters.push("title = ?");
            values.push(Box::new(title.to_string()));
        }
        if filters.is_empty() {
            Err(anyhow!("No filter provided for get_note"))
        } else {
            let filter = filters.join(" AND ");
            let query = format!(
                "SELECT id, content, title, link_id, created_at, modified_at
            FROM note
            WHERE {filter}",
            );
            let mut stmt = tx.prepare(&query)?;
            let mut rows = stmt.query(params_from_iter(values.iter()))?;
            let row0 = rows.next()?;
            if let Some(row) = row0 {
                let created_at: String = row.get(4)?;
                let modified_at: String = row.get(5)?;
                Ok(Some(super::Note {
                    id: row.get(0)?,
                    content: row.get(1)?,
                    title: row.get(2)?,
                    link_id: row.get(3)?,
                    created_at: created_at.parse()?,
                    modified_at: modified_at.parse()?,
                }))
            } else {
                Ok(None)
            }
        }
    }

    pub fn delete_note(tx: &Transaction, note_id: &TableId) -> Result<()> {
        // Our foreign key cascades will clean up tags -- a possible improvement
        // would be to remove orphaned tags after this is applied.
        let delete_query = "DELETE FROM note WHERE id = ?";
        tx.execute(delete_query, [&note_id])?;
        Ok(())
    }

    // SEARCH
    pub fn search_links(tx: &Transaction, term: &str) -> Result<Vec<super::Link>> {
        get_links(tx, vec![], Some(term))
    }
}

mod util {
    use anyhow::{anyhow, Result};

    pub fn slugify(tag: &str) -> Result<String> {
        let mut is_sep = true;
        let mut slug: String = "".to_string();
        tag.to_lowercase().trim().chars().for_each(|c| {
            if c.is_alphanumeric() {
                is_sep = false;
                slug.push(c);
            } else if c == ':' {
                slug.push(':');
            } else if !is_sep {
                slug.push('-');
                is_sep = true;
            }
        });
        let mut valid_pieces: Vec<String> = vec![];
        for piece in slug.split(":") {
            let s = piece.trim_matches('-');
            if s.is_empty() {
                return Err(anyhow!("Invalid tag `{}`", tag));
            } else {
                valid_pieces.push(s.to_string());
            }
        }
        if valid_pieces.is_empty() {
            return Err(anyhow!("Invalid tag `{}`", tag));
        }
        Ok(valid_pieces.join(":"))
    }

    #[test]
    fn test_slugify() -> Result<()> {
        let base_case = "Jacques Torneur";
        assert_eq!(slugify(base_case)?, "jacques-torneur".to_string());

        let alphanumeric = "Excuse 17";
        assert_eq!(slugify(alphanumeric)?, "excuse-17".to_string());

        let punctuated = "Mr. Bungle";
        assert_eq!(slugify(punctuated)?, "mr-bungle".to_string());

        let trim_whitespace = " Ursula K. Le Guin ";
        assert_eq!(slugify(trim_whitespace)?, "ursula-k-le-guin".to_string());

        let namespaced = "ns1:ns2:actual term";
        assert_eq!(slugify(namespaced)?, "ns1:ns2:actual-term".to_string());

        let trim_interior_whitespace = "  ns1  : ns2 ?: actual term";
        assert_eq!(
            slugify(trim_interior_whitespace)?,
            "ns1:ns2:actual-term".to_string()
        );

        let invalid_empty = "";
        assert!(slugify(invalid_empty).is_err());

        let invalid_whitespace_only = "   ";
        assert!(slugify(invalid_whitespace_only).is_err());

        let invalid_punctuation_only = "???";
        assert!(slugify(invalid_punctuation_only).is_err());

        let invalid_leading_namespace = ":foo";
        assert!(slugify(invalid_leading_namespace).is_err());

        let invalid_trailing_namespace = "foo:";
        assert!(slugify(invalid_trailing_namespace).is_err());

        let invalid_empty_namespace = "foo::bar";
        assert!(slugify(invalid_empty_namespace).is_err());

        let invalid_whitespace_namespace = "foo: :bar";
        assert!(slugify(invalid_whitespace_namespace).is_err());

        Ok(())
    }
}
