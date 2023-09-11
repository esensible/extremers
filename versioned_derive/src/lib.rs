extern crate proc_macro;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{DeriveInput, Data, Fields};


// use syn::{Variant, Ident};     DataEnum, DataStruct};




#[proc_macro_derive(Versioned)]
pub fn derive_versioned(input: TokenStream) -> TokenStream {
    let input = syn::parse_macro_input!(input as DeriveInput);

    // let name = &input.ident;
    let expanded = match &input.data {
        Data::Enum(_) => versioned_enum(input),
        Data::Struct(_) => versioned_struct(input),
        Data::Union(_) => panic!("Unions are not supported"),
    };

    expanded.into()
}

fn versioned_struct(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let versioned_name = format_ident!("Versioned{}", name);
    let delta_name = format_ident!("Delta{}", name);

    let (versioned_fields, delta_fields, new_initializers, get_initializers) = if let Data::Struct(data_struct) = &input.data {
        let mut versioned_fields = vec![];
        let mut delta_fields = vec![];
        let mut new_initializers = vec![];
        let mut get_initializers = vec![];

        for field in &data_struct.fields {
            let field_name = &field.ident;
            let field_type = &field.ty;
            
            versioned_fields.push(quote! {
                #field_name: VersionedValue<VersionedType<#field_type>>
            });
            delta_fields.push(quote! {
                #[serde(skip_serializing_if = "Option::is_none")]
                #field_name: DeltaType<#field_type>
            });
            new_initializers.push(quote! {
                #field_name: Versioned::new(value.#field_name, version)
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
        struct #delta_name {
            #(#delta_fields),*
        }

        impl Versioned for #name {
            type Value = #versioned_name;
            type Delta = #delta_name;

            fn new(value: Self, version: usize) -> VersionedValue<Self::Value> {
                VersionedValue {
                    value: Self::Value {
                        #(#new_initializers),*
                    },
                    version: version,
                }
            }

            fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaType<Self> {
                Some(Self::Delta {
                    #(#get_initializers),*
                })
            }
        }
    };

    expanded.into()
}

fn versioned_enum(input: syn::DeriveInput) -> proc_macro2::TokenStream {
    let name = &input.ident;

    let versioned_name = format_ident!("Versioned{}", name);
    let delta_name = format_ident!("Delta{}", name);

    let (versioned_variants, delta_variants, new_match_arms, get_match_arms) = if let Data::Enum(data_enum) = &input.data {
        let mut versioned_variants = vec![];
        let mut delta_variants = vec![];
        let mut new_match_arms = vec![];
        let mut get_match_arms = vec![];

        for variant in &data_enum.variants {
            let variant_name = &variant.ident;
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
                        versioned_fields.push(quote! {
                            #field_name: VersionedValue<VersionedType<#field_type>>
                        });
                        delta_fields.push(quote! {
                            #[serde(skip_serializing_if = "Option::is_none")]
                            #field_name: DeltaType<#field_type>
                        });
                        field_initializers.push(quote! {
                            #field_name: Versioned::new(#field_name, version)
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
        // #[serde(tag = stringify!(#name))]
        enum #delta_name {
            #(#delta_variants),*
        }

        impl Versioned for #name {
            type Value = #versioned_name;
            type Delta = #delta_name;

            fn new(original: Self, version: usize) -> VersionedValue<Self::Value> {
                let value = match original {
                    #(#new_match_arms),*
                };
                VersionedValue { value, version }
            }

            fn get(value: VersionedValue<Self::Value>, version: usize) -> DeltaType<Self> {
                match value.value {
                    #(#get_match_arms),*
                }
            }
        }
    };

    expanded.into()
}
