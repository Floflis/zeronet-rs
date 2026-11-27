#[macro_export]
macro_rules! impl_plugin {
    ($name:ident, $provider:ident, $plugin:expr, $permissions:expr) => {
        wai_bindgen_wasmer::import!($plugin);
        pub struct $name {
            pub store: wai_bindgen_wasmer::wasmer::Store,
            pub module: wai_bindgen_wasmer::wasmer::Module,
            pub provider: $provider,
        }

        pub fn load_plugins() -> mut_static::MutStatic<Vec<$name>> {
            let plugins = (&*$crate::environment::PLUGINS)
                .iter()
                .filter(|p| p.permissions.contains(&$permissions));
            let mut plugins_loaded = Vec::new();
            for plugin in plugins {
                let engine = wai_bindgen_wasmer::wasmer::sys::Cranelift::default();
                let mut store = wai_bindgen_wasmer::wasmer::Store::new(engine);
                let bytes = std::fs::read(&plugin.path).unwrap();
                let module = wai_bindgen_wasmer::wasmer::Module::new(&store, &bytes);
                if let Ok(module) = module {
                    let mut imports = wai_bindgen_wasmer::wasmer::imports! {};
                    let funs = $provider::instantiate(&mut store, &module, &mut imports);
                    if let Ok((provider, _)) = funs {
                        let plugin = $name {
                            store,
                            module,
                            provider,
                        };
                        plugins_loaded.push(plugin);
                    } else {
                        let error = funs.err().unwrap();
                        error!("Failed to load plugin {:?}", error);
                    }
                }
            }
            mut_static::MutStatic::from(plugins_loaded)
        }
    };
}
