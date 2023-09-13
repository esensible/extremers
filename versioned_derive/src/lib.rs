//! This module provides procedural macros for deriving fine grained (per field) versioned data structures.

extern crate proc_macro;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Data, Fields};


struct VersionedAttribute {
    meta: Vec<syn::NestedMeta>,
}


fn extract_versioned_attributes(attrs: &[syn::Attribute]) -> Vec<VersionedAttribute> {
    let mut extracted_attrs = Vec::new();

    for attr in attrs {
        if attr.path.is_ident("serde") {
            if let Ok(meta) = attr.parse_meta() {
                if let syn::Meta::List(meta_list) = meta {
                    extracted_attrs.push(VersionedAttribute {
                        meta: meta_list.nested.into_iter().collect(),
                    });
                }
            }
        }
    }

    extracted_attrs
}

/// The main procedural macro to derive the `Versioned` trait.
///
/// The macro supports both enums and structs and will generate the appropriate
/// versioned and delta types, along with the required implementations for the `Versioned` trait.
// #[proc_macro_derive(Versioned, attributes(serde))]
// pub fn derive_versioned(input: TokenStream) -> TokenStream {
//     let input = syn::parse_macro_input!(input as DeriveInput);

//     // let name = &input.ident;
//     let expanded = match &input.data {
//         Data::Enum(_) => versioned_enum(input),
//         Data::Struct(_) => versioned_struct(input),
//         Data::Union(_) => panic!("Unions are not supported"),
//     };

//     expanded.into()
// }


#[proc_macro_derive(Versioned, attributes(serde))]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    // Check if the `atomic` attribute is present
    let is_atomic = input.attrs.iter().any(|attr| {
        if let Ok(meta) = attr.parse_meta() {
            if let syn::Meta::List(meta_list) = meta {
                if meta_list.path.is_ident("serde") {
                    return meta_list.nested.iter().any(|nested_meta| {
                        if let syn::NestedMeta::Meta(nested) = nested_meta {
                            return nested.path().is_ident("atomic");
                        }
                        false
                    });
                }
            }
        }
        false
    });

    let expanded = if is_atomic {
        // If `atomic` was detected, just implement the Atomic trait
        let name = &input.ident;
        quote! {
            impl Atomic for #name {}
        }
    } else {
        // Otherwise, proceed with generating versioned code
        match &input.data {
            Data::Enum(_) => versioned_enum(input),
            Data::Struct(_) => versioned_struct(input),
            Data::Union(_) => panic!("Unions are not supported"),
        }
    };

    expanded.into()
}


/// Generates code for structs to implement the `Versioned` trait.
///
/// The generated code includes versioned structs with fields wrapped in `::versioned::VersionedValue`,
/// delta structs with fields wrapped in `::versioned::DeltaType`, and the necessary methods of the `Versioned` trait.
fn versioned_struct(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let versioned_name = format_ident!("Versioned{}", name);
    let delta_name = format_ident!("Delta{}", name);

    let attrs = extract_versioned_attributes(&input.attrs);
    let mut type_serde_attrs = vec![];
    for attr in &attrs {
        // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
        for nested_meta in &attr.meta {
            type_serde_attrs.push(quote! { #[serde(#nested_meta)] });
        }
    }

    let (versioned_fields, delta_fields, new_initializers, get_initializers) = if let Data::Struct(data_struct) = &input.data {
        let mut versioned_fields = vec![];
        let mut delta_fields = vec![];
        let mut new_initializers = vec![];
        let mut get_initializers = vec![];

        for field in &data_struct.fields {
            let field_name = &field.ident;
            let field_type = &field.ty;
            let field_attrs = extract_versioned_attributes(&field.attrs);

            let mut serde_attrs = vec![quote! { #[serde(skip_serializing_if = "Option::is_none")] }];
            for attr in &field_attrs {
                // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
                for nested_meta in &attr.meta {
                    serde_attrs.push(quote! { #[serde(#nested_meta)] });
                }
            }            
            versioned_fields.push(quote! {
                #field_name: ::versioned::VersionedValue<::versioned::VersionedType<#field_type>>
            });
            delta_fields.push(quote! {
                #(#serde_attrs)*
                #field_name: ::versioned::DeltaType<#field_type>
            });
            new_initializers.push(quote! {
                #field_name: ::versioned::Versioned::new(value.#field_name, version)
            });
            get_initializers.push(quote! {
                #field_name: #field_type::get(value.value.#field_name, version)
            });
        }

        (versioned_fields, delta_fields, new_initializers, get_initializers)
    } else {
        panic!("versioned_struct can only be applied to structs");
    };

    let expanded = quote! {
        struct #versioned_name {
            #(#versioned_fields),*
        }

        #[derive(Serialize)]
        #(#type_serde_attrs)*
        struct #delta_name {
            #(#delta_fields),*
        }

        impl ::versioned::Versioned for #name {
            type Value = #versioned_name;
            type Delta = #delta_name;

            fn new(value: Self, version: usize) -> ::versioned::VersionedValue<Self::Value> {
                ::versioned::VersionedValue {
                    value: Self::Value {
                        #(#new_initializers),*
                    },
                    version: version,
                }
            }

            fn get(value: ::versioned::VersionedValue<Self::Value>, version: usize) -> ::versioned::DeltaType<Self> {
                Some(Self::Delta {
                    #(#get_initializers),*
                })
            }
        }
    };

    expanded.into()
}


/// Generates code for enums to implement the `Versioned` trait.
///
/// The generated code includes versioned enums with variants wrapped in `::versioned::VersionedValue` (if necessary),
/// delta enums with variants wrapped in `::versioned::DeltaType` (if necessary), and the necessary methods of the `Versioned` trait.
fn versioned_enum(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let versioned_name = format_ident!("Versioned{}", name);
    let delta_name = format_ident!("Delta{}", name);

    let attrs = extract_versioned_attributes(&input.attrs);
    let mut type_serde_attrs = vec![];
    for attr in &attrs {
        // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
        for nested_meta in &attr.meta {
            type_serde_attrs.push(quote! { #[serde(#nested_meta)] });
        }
    }

    let (versioned_variants, delta_variants, new_match_arms, get_match_arms) = if let Data::Enum(data_enum) = &input.data {
        let mut versioned_variants = vec![];
        let mut delta_variants = vec![];
        let mut new_match_arms = vec![];
        let mut get_match_arms = vec![];

        for variant in &data_enum.variants {
            let variant_name = &variant.ident;
            let variant_attrs = extract_versioned_attributes(&variant.attrs);
            let mut variant_serde_attrs = vec![];
            for attr in &variant_attrs {
                for nested_meta in &attr.meta {
                    variant_serde_attrs.push(quote! { #[serde(#nested_meta)] });
                }
            }

            match &variant.fields {
                Fields::Named(fields_named) => {
                    let mut versioned_fields = vec![];
                    let mut delta_fields = vec![];
                    let mut field_initializers = vec![];
                    let mut delta_initializers = vec![];
                    let mut field_names = vec![];

                    for field in &fields_named.named {
                        let field_name = &field.ident;
                        let field_type = &field.ty;
                        let field_attrs = extract_versioned_attributes(&field.attrs);

                        let mut serde_attrs = vec![quote! { #[serde(skip_serializing_if = "Option::is_none")] }];
                        for attr in &field_attrs {
                            // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
                            for nested_meta in &attr.meta {
                                serde_attrs.push(quote! { #[serde(#nested_meta)] });
                            }
                        }            
            
                        versioned_fields.push(quote! {
                            #field_name: ::versioned::VersionedValue<::versioned::VersionedType<#field_type>>
                        });
                        delta_fields.push(quote! {
                            #(#serde_attrs)*
                            #field_name: ::versioned::DeltaType<#field_type>
                        });
                        field_initializers.push(quote! {
                            #field_name: ::versioned::Versioned::new(#field_name, version)
                        });
                        delta_initializers.push(quote! {
                            #field_name: #field_type::get(#field_name, version)
                        });
                        field_names.push(field_name);
                    }

                    versioned_variants.push(quote! {
                        #variant_name { #(#versioned_fields),* }
                    });
                    delta_variants.push(quote! {
                        #(#variant_serde_attrs)*
                        #variant_name { #(#delta_fields),* }
                    });
                    new_match_arms.push(quote! {
                        #name::#variant_name { #(#field_names),* } => #versioned_name::#variant_name { #(#field_initializers),* }
                    });
                    get_match_arms.push(quote! {
                        #versioned_name::#variant_name { #(#field_names),* } => Some(#delta_name::#variant_name { #(#delta_initializers),* })
                    });
                }
                Fields::Unit => {
                    versioned_variants.push(quote! {
                        #variant_name
                    });
                    delta_variants.push(quote! {
                        #(#variant_serde_attrs)*
                        #variant_name
                    });
                    new_match_arms.push(quote! {
                        #name::#variant_name => #versioned_name::#variant_name
                    });
                    get_match_arms.push(quote! {
                        #versioned_name::#variant_name => Some(#delta_name::#variant_name)
                    });
                }
                _ => {
                    panic!("Unsupported variant type");
                }
            }
        }

        (versioned_variants, delta_variants, new_match_arms, get_match_arms)
    } else {
        panic!("Versioned can only be derived for enums");
    };

    let expanded = quote! {
        enum #versioned_name {
            #(#versioned_variants),*
        }

        #[derive(Serialize)]
        #(#type_serde_attrs)*
        enum #delta_name {
            #(#delta_variants),*
        }

        impl ::versioned::Versioned for #name {
            type Value = #versioned_name;
            type Delta = #delta_name;

            fn new(original: Self, version: usize) -> ::versioned::VersionedValue<Self::Value> {
                let value = match original {
                    #(#new_match_arms),*
                };
                ::versioned::VersionedValue { value, version }
            }

            fn get(value: ::versioned::VersionedValue<Self::Value>, version: usize) -> ::versioned::DeltaType<Self> {
                match value.value {
                    #(#get_match_arms),*
                }
            }
        }
    };

    expanded.into()
}
