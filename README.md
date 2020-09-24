# Crown Controller
 
This program can be used for managing crown actions on Logitech Craft keyboard under Linux.

Actions can be defined in `yaml` file that should be stored in `~/.config/crown-controller/config.yaml`,
and example `config.yaml` is available in this repository. 

To build this program you need to have `Rust` available on your system, calling
```
cargo build --release
```
in copy of this repository should generate binary in target/release/crown-controller
