//! `#[pitwall_extension::tools]`: a tile's declared agent tools from an
//! impl block (docs/spec/tile-sdk/agent-tools.md §11.3). Each method
//! marked `#[tool(kind = "show")]` or `#[tool(kind = "present")]` becomes
//! the tool of its name; the attribute writes
//! `impl pitwall_extension::tools::Tools for <Type>`, which calls it with
//! the call's input read as the method's `Call<Input>`.

use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, spanned::Spanned, Error, ImplItem, ItemImpl, LitStr};

/// See the crate's documentation (and `pitwall_extension::tools`).
#[proc_macro_attribute]
pub fn tools(attr: TokenStream, item: TokenStream) -> TokenStream {
    if !attr.is_empty() {
        return Error::new(proc_macro2::TokenStream::from(attr).span(), "#[tools] takes no arguments").to_compile_error().into();
    }
    let mut block = parse_macro_input!(item as ItemImpl);
    match expand(&mut block) {
        Ok(t) => t.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn expand(block: &mut ItemImpl) -> syn::Result<proc_macro2::TokenStream> {
    if let Some((_, path, _)) = &block.trait_ {
        return Err(Error::new(path.span(), "#[tools] goes on the tile's own impl block (`impl MyTile { … }`), not a trait impl"));
    }
    let mut arms = Vec::new();
    let mut list = Vec::new();
    for item in &mut block.items {
        let ImplItem::Fn(f) = item else { continue };
        let Some(at) = f.attrs.iter().position(|a| a.path().is_ident("tool")) else { continue };
        let attr = f.attrs.remove(at);
        let mut kind = None;
        attr.parse_nested_meta(|m| {
            if m.path.is_ident("kind") {
                let k: LitStr = m.value()?.parse()?;
                kind = Some(k);
                Ok(())
            } else {
                Err(m.error("#[tool] takes kind = \"show\" or \"present\""))
            }
        })?;
        let kind = kind.ok_or_else(|| Error::new(attr.span(), "#[tool] needs kind = \"show\" or \"present\""))?;
        let k = match kind.value().as_str() {
            "show" => quote!(::pitwall_extension::tools::Kind::Show),
            "present" => quote!(::pitwall_extension::tools::Kind::Present),
            "read" | "act" => return Err(Error::new(kind.span(), "read and act tools come in a later SDK (agent-tools.md §11.2); this one has show and present")),
            other => return Err(Error::new(kind.span(), format!("no kind “{other}”: show or present"))),
        };
        let name = f.sig.ident.clone();
        let text = name.to_string();
        if text.len() > 32 || !text.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return Err(Error::new(name.span(), "a tool's name is lowercase letters, digits and _, at most 32"));
        }
        if f.sig.inputs.len() != 3 {
            return Err(Error::new(f.sig.span(), "a tool method takes (&mut self, call: Call<Input>, cx: &mut Cx<Self>)"));
        }
        list.push(quote!((#text, #k)));
        arms.push(quote! {
            #text => ::core::option::Option::Some(match call.typed() {
                ::core::result::Result::Ok(c) => self.#name(c, cx),
                ::core::result::Result::Err(e) => ::core::result::Result::Err(e),
            }),
        });
    }
    if list.is_empty() {
        return Err(Error::new(block.self_ty.span(), "#[tools] found no #[tool(kind = …)] method"));
    }
    let ty = &block.self_ty;
    let (ig, tg, wc) = block.generics.split_for_impl();
    Ok(quote! {
        #block

        impl #ig ::pitwall_extension::tools::Tools for #ty #tg #wc {
            const TOOLS: &'static [(&'static str, ::pitwall_extension::tools::Kind)] = &[#(#list),*];

            fn call_tool(&mut self, name: &str, call: ::pitwall_extension::tools::RawCall, cx: &mut ::pitwall_extension::Cx<Self>) -> ::core::option::Option<::core::result::Result<::pitwall_extension::tools::Reply, ::pitwall_extension::tools::ToolError>> {
                match name {
                    #(#arms)*
                    _ => ::core::option::Option::None,
                }
            }
        }
    })
}
