# 😼meowpad: a nice way to scratch

`meowpad` is a command-line scratchpad designed for capturing links and notes.
I often find myself recording a number of links for things I'm researching (or
just find interesting to read about) which I want to record and group together
but don't necessarily rise to the level of something worth sharing publicly.

My solution has always been to drop this in a text file or scratch entry in my
notes application called something like "linkdump" or "scratch" (at some
point, my "scratch" note got renamed to "meow"). This works, sort of: I have a
record of what I was reading, but I've often lost the context of when I was
reading it, why, and what's interesting about it, and as dozens or hundreds of
notes accumulate, they become increasingly useless as anything but a write-only
log.

`meowpad` is an attempt to put some structure on these sort of scratchpad notes,
revolving around my particular workflows. Its biggest inspiration is the Python
bookmark manager [`buku`](https://github.com/jarun/buku).

## Example Use

Suppose you are working on a command-line scratchpad designed for capturing
links and notes. You are trying to find a well-supported
[Readability](https://github.com/mozilla/readability) library. Fortunately
someone has done a bake-off recently and blogged about it!

```
meowpad add https://emschwartz.me/comparing-13-rust-crates-for-extracting-text-from-html/ \
    -t rust -t readability -t project:meowpad \
    -m "Text extraction shootout; the recommended projects are fast_html2md and dom_smoothie"
```

`rust`, `readability`, and `project:meowpad` are _tags_ for that link. If later
you want to mention other projects discussed in the link, fire up `$VISUAL`
with:

```
meowpad note https://emschwartz.me/comparing-13-rust-crates-for-extracting-text-from-html/
```

Maybe the RSS feed project mentioned by the author of that blog sounds interesting?

```
meowpad add https://scour.ing/about -t rss \
    --related-link https://emschwartz.me/comparing-13-rust-crates-for-extracting-text-from-html/ \
    --relation via
```

## Anti-goals

`meowpad` is *not* meant to be either a web-based bookmarks manager such as
Pinboard or Shiori or a read-later application such as Wallabag. It isn't meant
to record full web pages, the way [ArchiveBox](https://archivebox.io) or
[`monolith`](https://github.com/Y2Z/monolith) do, although finding clean
mechanisms for handing scratch links off to those sorts of tools is absolutely
within scope. Nor is `meowpad` meant to be a general-purpose mind mapper tool;
it is not meant to provide anything like the feature set of
[Obsidian](https://obsidian.md). (Exporting to Obsidian vaults, however, is
within scope.)
