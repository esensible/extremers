//! This module provides procedural macros for deriving fine grained (per field) versioned data structures.

extern crate proc_macro;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Data, Fields};


struct VersionedAttribute {
    meta: Vec<syn::NestedMeta>,
}


fn extract_serde_attributes(attrs: &[syn::Attribute]) -> Vec<VersionedAttribute> {
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
//         Data::Struct(_) => delta_struct(input),
//         Data::Union(_) => panic!("Unions are not supported"),
//     };

//     expanded.into()
// }


#[proc_macro_derive(Delta, attributes(serde, delta))]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    let expanded = match &input.data {
        Data::Enum(_) => versioned_enum(input),
        Data::Struct(_) => delta_struct(input),
        Data::Union(_) => panic!("Unions are not supported"),
    };

    expanded.into()
}


/// Generates code for structs to implement the `Versioned` trait.
///
/// The generated code includes versioned structs with fields wrapped in `::versioned::VersionedValue`,
/// delta structs with fields wrapped in `::versioned::DeltaType`, and the necessary methods of the `Versioned` trait.
fn delta_struct(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let visibility = &input.vis;

    let delta_name = format_ident!("Delta{}", name);

    let attrs = extract_serde_attributes(&input.attrs);
    let mut type_serde_attrs = vec![];
    for attr in &attrs {
        // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
        for nested_meta in &attr.meta {
            type_serde_attrs.push(quote! { #[serde(#nested_meta)] });
        }
    }

    let (delta_fields, delta_field_ctors) = if let Data::Struct(data_struct) = &input.data {
        let mut delta_fields = vec![];
        let mut delta_field_ctors = vec![];

        for field in &data_struct.fields {
            let field_name = &field.ident;
            let field_type = &field.ty;
            let field_attrs = extract_serde_attributes(&field.attrs);

            let mut serde_attrs = vec![quote! { #[serde(skip_serializing_if = "Option::is_none")] }];
            for attr in &field_attrs {
                // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
                for nested_meta in &attr.meta {
                    serde_attrs.push(quote! { #[serde(#nested_meta)] });
                }
            }            
            delta_fields.push(quote! {
                #(#serde_attrs)*
                #field_name: Option<<#field_type as ::versioned::DeltaTrait>::Type>
            });
            delta_field_ctors.push(quote! {
                #field_name: <#field_type as ::versioned::DeltaTrait>::delta(&lhs.#field_name, &rhs.#field_name)
            });
        }

        (delta_fields, delta_field_ctors)
    } else {
        panic!("delta_struct can only be applied to structs");
    };

    let expanded = quote! {
        #[derive(Serialize, Clone)]
        #(#type_serde_attrs)*
        #visibility struct #delta_name {
            #(#delta_fields),*
        }

        impl ::versioned::DeltaTrait for #name {
            type Type = #delta_name;

            fn delta(lhs: &#name, rhs: &#name) -> Option<Self::Type> {
                if lhs == rhs {
                    return None;
                }
        
                Some(Self::Type {
                    #(#delta_field_ctors),*
                })
            }
        }
    };
    println!("## {}", name);
    println!("{}", expanded.to_string());

    expanded.into()
}


/// Generates code for enums to implement the `Versioned` trait.
///
/// The generated code includes versioned enums with variants wrapped in `::versioned::VersionedValue` (if necessary),
/// delta enums with variants wrapped in `::versioned::DeltaType` (if necessary), and the necessary methods of the `Versioned` trait.
fn versioned_enum(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;
    let visibility = &input.vis;
    let delta_name = format_ident!("Delta{}", name);

    let attrs = extract_serde_attributes(&input.attrs);
    let mut type_serde_attrs = vec![];
    for attr in &attrs {
        // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
        for nested_meta in &attr.meta {
            type_serde_attrs.push(quote! { #[serde(#nested_meta)] });
        }
    }

    let (delta_variants, delta_match_arms) = if let Data::Enum(data_enum) = &input.data {
        let mut delta_variants = vec![];
        let mut delta_match_arms = vec![];
        let mut lhs_cross_arms = vec![];
        let mut rhs_cross_arms = vec![];

        for variant in &data_enum.variants {
            let variant_name = &variant.ident;
            let variant_attrs = extract_serde_attributes(&variant.attrs);
            let mut variant_serde_attrs = vec![];
            for attr in &variant_attrs {
                for nested_meta in &attr.meta {
                    variant_serde_attrs.push(quote! { #[serde(#nested_meta)] });
                }
            }

            let is_skip_fields = variant.attrs.iter().any(|attr| {
                attr.path.is_ident("delta") && attr.tokens.to_string().contains("skip_fields")
            });

            match &variant.fields {
                Fields::Named(fields_named) => {
                    let mut delta_field_names = vec![];
                    let mut delta_fields = vec![];
                    let mut delta_field_ctors = vec![];
                    let mut delta_field_clones = vec![];

                    for field in &fields_named.named {
                        let field_name = field.ident.as_ref().unwrap();
                        let field_type = &field.ty;
                        let field_attrs = extract_serde_attributes(&field.attrs);

                        let mut serde_attrs = vec![quote! { #[serde(skip_serializing_if = "Option::is_none")] }];
                        for attr in &field_attrs {
                            // For now, assuming the nested attributes from `serde` can be directly translated to `serde`.
                            for nested_meta in &attr.meta {
                                serde_attrs.push(quote! { #[serde(#nested_meta)] });
                            }
                        }            

                        let is_skip_field = field.attrs.iter().any(|attr| {
                            attr.path.is_ident("delta") && attr.tokens.to_string().contains("skip")
                        });
                       
                        if !is_skip_fields && !is_skip_field{
                            delta_field_names.push(field_name);

                            delta_fields.push(quote! {
                                #(#serde_attrs)*
                                #field_name: Option<<#field_type as ::versioned::DeltaTrait>::Type>
                            });
               
                            let field_name_lhs = format_ident!("lhs_{}", field_name);
                            let field_name_rhs = format_ident!("rhs_{}", field_name);

                            delta_field_ctors.push(quote! {
                                #field_name: #field_type::delta(&#field_name_lhs, &#field_name_rhs)
                            });
                            
                            delta_field_clones.push(quote! {
                                #field_name: Some(#field_name_rhs.clone())
                            });
                        }
                    }

                    if is_skip_fields || delta_field_names.is_empty() {
                        delta_variants.push(quote! {
                            #(#variant_serde_attrs)*
                            #variant_name
                        });
                        println!("Skipping fields for variant {}::{}", delta_name, variant_name);
                        delta_match_arms.push(quote! {
                            (Self::#variant_name { .. }, Self::#variant_name { .. }) if (lhs == rhs) => None,
                            (Self::#variant_name { .. }, Self::#variant_name { .. }) => Some(Self::Type::#variant_name)
                        });

                        lhs_cross_arms.push((variant_name, quote! {
                            { .. }
                        }));
                        rhs_cross_arms.push((
                            variant_name, 
                            quote! {{ .. }},
                            quote! {},
                        ));
                    } else {
                        let mut ref_fields: Vec<_> = delta_field_names.iter().map(|f| quote! { ref #f }).collect();
                        if delta_field_names.len() != fields_named.named.len() {
                            ref_fields.push(quote! { .. });
                        }
                        // TODO: I think we should be using ref_fields below
                        delta_variants.push(quote! {
                            #(#variant_serde_attrs)*
                            #variant_name { #(#delta_fields),* }
                        });

                        let lhs_names: Vec<_> = delta_field_names.iter()
                        .map(|name| {
                            let lhs_name = format_ident!("lhs_{}", name);
                            quote! { #name: #lhs_name }
                        })
                        .collect();
                    
                        let rhs_names: Vec<_> = delta_field_names.iter()
                        .map(|name| {
                            let rhs_name = format_ident!("rhs_{}", name);
                            quote! { #name: #rhs_name }
                        })
                        .collect();

                        delta_match_arms.push(quote! {
                            (Self::#variant_name { .. }, Self::#variant_name { .. }) if (lhs == rhs) => None,

                            (Self::#variant_name { #(#lhs_names),*  }, Self::#variant_name { #(#rhs_names),* }) => Some(Self::Type::#variant_name {
                                #(#delta_field_ctors),*
                            })
                
                        });

                        lhs_cross_arms.push((variant_name, quote! {
                            { .. }
                        }));
                        rhs_cross_arms.push((
                            variant_name, 
                            quote! { { #(#rhs_names),* }},
                            quote! { { #(#delta_field_clones),* } },
                        ));

                    }
                }
                Fields::Unit => {
                    delta_variants.push(quote! {
                        #(#variant_serde_attrs)*
                        #variant_name
                    });
                    delta_match_arms.push(quote! {
                        (Self::#variant_name, Self::#variant_name) => None
                    });
                    lhs_cross_arms.push((variant_name, quote! { }));
                    rhs_cross_arms.push((
                        variant_name, 
                        quote! {},
                        quote! {},
                    ));
                }
                _ => {
                    panic!("Unsupported variant type");
                }
            }
        }

        for (lhs_name, lhs_match) in &lhs_cross_arms {
            for (rhs_name, rhs_match, rhs_init) in &rhs_cross_arms {
                if lhs_name != rhs_name {

                    delta_match_arms.push(quote! {
                        (Self::#rhs_name #rhs_match, Self::#lhs_name #lhs_match) => Some(Self::Type::#rhs_name #rhs_init)
                    });
                }
            }
        }
        (delta_variants, delta_match_arms)
    } else {
        panic!("Versioned can only be derived for enums");
    };

    let expanded = quote! {
        #[derive(Serialize, Clone)]
        #(#type_serde_attrs)*
        #visibility enum #delta_name {
            #(#delta_variants),*
        }

        impl ::versioned::DeltaTrait for #name {
            type Type = #delta_name;

            fn delta(lhs: &E, rhs: &E) -> Option<Self::Type> {
                match (lhs, rhs) {
                    #(#delta_match_arms),*,
                    (_, _) => None
                }
            }       
        }
    };

    println!("## {}", name);
    println!("{}", expanded.to_string());

    expanded.into()
}
