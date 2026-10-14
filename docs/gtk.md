# Previous GTK frontend

The GTK 4 / libadwaita frontend (`crates/rustxt-gtk`, the `rustxt` binary of
releases 0.4.x) was removed from the tree in 0.6.0. The shipped application is
the Iced frontend described in the [README](../README.md).

The last revision that contains the GTK crate, its end-to-end tests, and its
documentation is preserved at the git tag `v0.4.1-gtk`:

```sh
git checkout v0.4.1-gtk
cargo run -p rustxt            # needs GTK 4.16, libadwaita 1.6 and GtkSourceView 5 dev packages
```

That tag's `docs/gtk.md` holds the GTK-era user documentation (features,
packages, build instructions, configuration, and shortcuts).
