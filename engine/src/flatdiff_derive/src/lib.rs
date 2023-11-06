//! This module provides procedural macros for deriving fine grained (per field) flatdiff data structures.

extern crate proc_macro;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{Data, DeriveInput, Fields};

#[proc_macro_derive(FlatDiffSer, attributes(serde, delta))]
pub fn derive_flatdiff(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    let expanded = match &input.data {
        Data::Enum(_) => flatdiff_enum(input),
        Data::Struct(_) => delta_struct(input),
        Data::Union(_) => panic!("Unions are not supported"),
    };

    expanded.into()
}

/// Generates code for structs to implement the `Versioned` trait.
///
/// The generated code includes flatdiff structs with fields wrapped in `engine::VersionedValue`,
/// delta structs with fields wrapped in `engine::DeltaType`, and the necessary methods of the `Versioned` trait.
fn delta_struct(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let (field_diffs, field_flattens, field_counts) = if let Data::Struct(data_struct) = &input.data
    {
        let mut field_diffs = vec![];
        let mut field_flattens = vec![];
        let mut field_counts = vec![];

        for field in &data_struct.fields {
            let field_name = &field.ident;
            let field_type = &field.ty;

            field_diffs.push(quote! {
                <#field_type as engine::FlatDiffSer>::diff::<S>(&self.#field_name, &rhs.#field_name, stringify!(#field_name), state)?;
            });
            field_flattens.push(quote! {
                <#field_type as engine::FlatDiffSer>::flatten::<S>(&self.#field_name, stringify!(#field_name), state)?;
            });
            field_counts.push(quote! {
                count += <#field_type as engine::FlatDiffSer>::count();
            });
        }

        (field_diffs, field_flattens, field_counts)
    } else {
        panic!("delta_struct can only be applied to structs");
    };

    let expanded = quote! {
        impl  engine::FlatDiffSer for #name
        {
            fn diff<S>(&self, rhs: &Self, label: &'static str, state: &mut S::SerializeStruct) -> Result<(), S::Error>
            where
                S: Serializer
            {
                #(#field_diffs)*
                Ok(())
            }

            fn flatten<S>(&self, label: &'static str, state: &mut S::SerializeStruct) -> Result<(), S::Error>
            where
                S: Serializer
            {
                #(#field_flattens)*
                Ok(())
            }

            fn count() -> usize {
                let mut count = 0;
                #(#field_counts)*
                count
            }
        }
    };
    // println!("## {}", name);
    // println!("{}", expanded.to_string());

    expanded
}

/// Generates code for enums to implement the `Versioned` trait.
///
/// The generated code includes flatdiff enums with variants wrapped in `engine::VersionedValue` (if necessary),
/// delta enums with variants wrapped in `engine::DeltaType` (if necessary), and the necessary methods of the `Versioned` trait.
fn flatdiff_enum(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let (diff_match_arms, flatten_match_arms) = if let Data::Enum(data_enum) = &input.data {
        let mut diff_match_arms = vec![];
        let mut flatten_match_arms = vec![];
        let mut cross_terms = vec![];

        for variant in &data_enum.variants {
            let variant_name = &variant.ident;

            match &variant.fields {
                Fields::Named(fields_named) => {
                    // let mut field_names = vec![];
                    // let mut delta_fields = vec![];
                    let mut lhs_match_fields = vec![];
                    let mut rhs_match_fields = vec![];
                    let mut diff_fields = vec![];
                    let mut cross_diff_fields = vec![];
                    let mut flatten_fields = vec![];

                    for field in &fields_named.named {
                        let field_name = field.ident.as_ref().unwrap();
                        let field_type = &field.ty;

                        let field_name_lhs = format_ident!("lhs_{}", field_name);
                        let field_name_rhs = format_ident!("rhs_{}", field_name);

                        lhs_match_fields.push(quote! { #field_name: #field_name_lhs });
                        rhs_match_fields.push(quote! { #field_name: #field_name_rhs });

                        diff_fields.push(quote! {
                            <#field_type as engine::FlatDiffSer>::diff::<S>(#field_name_lhs, #field_name_rhs, stringify!(#field_name), state)?;
                        });

                        cross_diff_fields.push(quote! {
                            <#field_type as engine::FlatDiffSer>::flatten::<S>(self, label, state)?;
                        });

                        flatten_fields.push(quote! {
                            <#field_type as engine::FlatDiffSer>::flatten::<S>(#field_name_lhs, stringify!(#field_name), state)?;
                        });
                    }

                    flatten_match_arms.push(quote! {
                        #name::#variant_name{#(#lhs_match_fields),*} => {
                            state.serialize_field(label, stringify!(#variant_name))?;
                            #(#flatten_fields)*
                        }
                    });

                    diff_match_arms.push(quote! {
                        (#name::#variant_name{#(#lhs_match_fields),*}, #name::#variant_name{#(#rhs_match_fields),*}) => {
                            #(#diff_fields)*
                        }
                    });

                    cross_terms.push((variant_name, quote! { { .. } }));
                }
                Fields::Unit => {
                    flatten_match_arms.push(quote! {
                        Self::#variant_name => {
                            state.serialize_field(label, stringify!(#variant_name))?;
                        }
                    });

                    diff_match_arms.push(quote! {
                        (Self::#variant_name, Self::#variant_name) => {}
                    });

                    cross_terms.push((variant_name, quote! {}));
                }
                _ => {
                    panic!("Unsupported variant type");
                }
            }
        }

        let mut cross_match_arms = vec![];
        for (lhs_name, lhs_match) in &cross_terms {
            for (rhs_name, rhs_match) in &cross_terms {
                if lhs_name != rhs_name {
                    cross_match_arms.push(quote! {
                        (#name::#lhs_name #lhs_match, #name::#rhs_name #rhs_match)
                    });
                }
            }
        }

        diff_match_arms.push(quote! {
            #(#cross_match_arms)|* => {
                self.flatten::<S>(label, state)?;
            }
        });

        (diff_match_arms, flatten_match_arms)
    } else {
        panic!("Versioned can only be derived for enums");
    };

    let expanded = quote! {
        impl engine::FlatDiffSer for #name
        {
            fn diff<S>(&self, rhs: &Self, label: &'static str, state: &mut S::SerializeStruct) -> Result<(), S::Error>
            where
                S: Serializer

            {
                match (self, rhs) {
                    #(#diff_match_arms),*
                };
                Ok(())
            }

            fn flatten<S>(&self, label: &'static str, state: &mut S::SerializeStruct) -> Result<(), S::Error>
            where
                S: Serializer
            {
                match self {
                    #(#flatten_match_arms),*
                };
                Ok(())
            }

            fn count() -> usize {
                1
            }
        }
    };

    // println!("## {}", name);
    // println!("{}", expanded.to_string());

    expanded
}
