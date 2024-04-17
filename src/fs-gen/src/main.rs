use std::{fs::remove_dir_all, path::Path, str::FromStr};

use image_builder::merge_layer;
use crate::initramfs_generator::{create_init_file, generate_initramfs};

mod cli_args;
mod image_builder;
mod image_loader;
mod initramfs_generator;

fn main() {
    let args = cli_args::CliArgs::get_args();
    println!("Hello, world!, {:?}", args);

    // TODO: better organise layers and OverlayFS build in the temp directory
    match image_loader::download_image_fs(&args.image_name, args.temp_directory.clone()) {
        Err(e) => {
            eprintln!("Error: {}", e);
            return;
        },
        Ok(layers_paths) => {
            println!("Image downloaded successfully! Layers' paths:");
            for path in &layers_paths {
                println!(" - {}", path.display());
            }

            // FIXME: use a subdir of the temp directory instead
            let path = Path::new("/tmp/cloudlet");

            merge_layer(&layers_paths, path);
            create_init_file(path);
            generate_initramfs(path, Path::new(args.output_file.as_path()));
        }
    }

    // cleanup of temporary directory
    remove_dir_all(args.temp_directory.clone()).expect("Could not remove temporary directory");
}