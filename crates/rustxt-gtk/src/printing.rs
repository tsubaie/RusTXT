//! Printing through GTK's native print dialog with GtkSourceView's compositor.

use gtk::prelude::*;
use sourceview5::prelude::*;

pub fn print(parent: &impl IsA<gtk::Window>, view: &sourceview5::View, title: &str) {
    let operation = gtk::PrintOperation::new();
    operation.set_job_name(title);
    operation.set_embed_page_setup(true);

    let compositor = sourceview5::PrintCompositor::from_view(view);
    compositor.set_wrap_mode(gtk::WrapMode::WordChar);
    compositor.set_print_line_numbers(0);
    compositor.set_header_format(true, Some(title), None, None);
    compositor.set_print_header(true);
    compositor.set_footer_format(true, None, Some("Page %N of %Q"), None);
    compositor.set_print_footer(true);

    let paginate = compositor.clone();
    operation.connect_begin_print(move |operation, context| {
        while !paginate.paginate(context) {}
        operation.set_n_pages(paginate.n_pages());
    });
    let draw = compositor.clone();
    operation.connect_draw_page(move |_, context, page| draw.draw_page(context, page));

    if let Err(error) = operation.run(gtk::PrintOperationAction::PrintDialog, Some(parent)) {
        eprintln!("RusTXT: printing failed: {error}");
    }
}
