# Third-party notices — 3D Viewer 0.2.2

3D Viewer uses third-party Rust crates listed in `Cargo.toml` and their transitive dependencies.

## cadrum / OpenCASCADE Technology

`cadrum` is licensed under the MIT License.

Release binaries that use the CAD import path include statically linked OpenCASCADE Technology (OCCT). OCCT is licensed under LGPL 2.1. Distributors of compiled binaries are responsible for satisfying the applicable LGPL requirements, including the ability for recipients to rebuild/relink the LGPL component as required by the license.

The source project intentionally uses the standard `cadrum` build path so the OpenCASCADE component can be rebuilt from source/relinked rather than depending on a private runtime DLL.

For authoritative license text and current redistribution requirements, consult the licenses shipped by the respective upstream projects before distribution.
