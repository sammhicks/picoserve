use proc_macro::TokenStream;
use quote::{quote, ToTokens};
use syn::{parse_macro_input, spanned::Spanned, DeriveInput};

trait HasAttributes: Spanned {
    fn attributes(&self) -> &[syn::Attribute];
}

impl HasAttributes for syn::DeriveInput {
    fn attributes(&self) -> &[syn::Attribute] {
        &self.attrs
    }
}

impl HasAttributes for syn::Variant {
    fn attributes(&self) -> &[syn::Attribute] {
        &self.attrs
    }
}

fn single_field(fields: &syn::Fields) -> Option<proc_macro2::TokenStream> {
    match fields {
        syn::Fields::Named(fields) => {
            let mut fields = fields.named.iter();
            let field = fields.next()?;
            fields
                .next()
                .is_none()
                .then(|| quote! { { #field: ref field } })
        }
        syn::Fields::Unnamed(fields) => {
            let mut fields = fields.unnamed.iter();
            let _field = fields.next()?;
            fields.next().is_none().then(|| quote! { (ref field) })
        }
        syn::Fields::Unit => None,
    }
}

enum StatusCodeAttribute {
    StatusCode(syn::Path),
    Transparent,
}

impl StatusCodeAttribute {
    fn parse<T: HasAttributes>(obj: &T) -> syn::Result<Option<Self>> {
        obj.attributes()
            .iter()
            .find(|attribute| attribute.path().is_ident("status_code"))
            .map(|status_code| {
                let syn::Meta::List(syn::MetaList { tokens, .. }) = &status_code.meta else {
                    return Err(syn::Error::new(
                        obj.span(),
                        "status_code attr must be in the form #[status_code(...)]",
                    ));
                };

                let path = syn::parse2::<syn::Path>(tokens.clone())?;

                Ok(if path.is_ident("transparent") {
                    StatusCodeAttribute::Transparent
                } else {
                    StatusCodeAttribute::StatusCode(
                        syn::parse_quote! { picoserve::response::StatusCode::#path },
                    )
                })
            })
            .transpose()
    }
}

fn try_derive_error_with_status_code(
    input: &DeriveInput,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let ident = &input.ident;

    let default_status_code = StatusCodeAttribute::parse(input)?;

    let status_code: syn::Expr = match &input.data {
        syn::Data::Struct(data_struct) => match default_status_code
            .ok_or_else(|| syn::Error::new(input.span(), "Missing #[status_code(..)]"))?
        {
            StatusCodeAttribute::StatusCode(path) => syn::Expr::Path(syn::ExprPath {
                attrs: Vec::new(),
                qself: None,
                path,
            }),
            StatusCodeAttribute::Transparent => {
                let fields = single_field(&data_struct.fields).ok_or_else(|| {
                    syn::Error::new(input.span(), "Transparent errors must have a single field")
                })?;

                syn::parse_quote! {
                    let Self #fields = self;
                    picoserve::response::ErrorWithStatusCode::status_code(field)
                }
            }
        },
        syn::Data::Enum(data_enum) => {
            let cases = data_enum
                .variants
                .iter()
                .map(|variant| {
                    let variant_status_code = StatusCodeAttribute::parse(variant)?;

                    let selected_status_code = variant_status_code
                        .as_ref()
                        .or(default_status_code.as_ref());

                    let selected_status_code = selected_status_code.ok_or_else(|| {
                        syn::Error::new(
                            variant.span(),
                            "Either the enum or this variant must have an attribute of status_code",
                        )
                    })?;

                    let ident = &variant.ident;
                    let fields;
                    let status_code: syn::Expr;

                    match selected_status_code {
                        StatusCodeAttribute::StatusCode(selected_status_code) => {
                            fields = match variant.fields {
                                syn::Fields::Named(..) => quote! { {..} },
                                syn::Fields::Unnamed(..) => quote! { (..) },
                                syn::Fields::Unit => quote! {},
                            };

                            status_code = syn::parse_quote! { #selected_status_code };
                        }
                        StatusCodeAttribute::Transparent => {
                            fields = single_field(&variant.fields).ok_or_else(|| {
                                syn::Error::new(
                                    variant.span(),
                                    "Transparent errors must have a single field",
                                )
                            })?;

                            status_code = syn::parse_quote! {
                                picoserve::response::ErrorWithStatusCode::status_code(field)
                            };
                        }
                    }

                    Ok(quote! { Self::#ident #fields => #status_code })
                })
                .collect::<Result<Vec<_>, syn::Error>>()?;

            syn::parse_quote! {
                match *self {
                    #(#cases,)*
                }
            }
        }
        syn::Data::Union(..) => {
            return Err(syn::Error::new(input.span(), "Must be a struct or an enum"))
        }
    };

    let syn::Generics {
        lt_token,
        params: generics_params,
        gt_token,
        where_clause,
    } = &input.generics;

    let self_is_display = syn::parse_quote!(Self: core::fmt::Display);

    let where_clause_predicates = where_clause
        .as_ref()
        .map(|where_clause| where_clause.predicates.iter())
        .into_iter()
        .flatten()
        .chain(std::iter::once(&self_is_display))
        .collect::<syn::punctuated::Punctuated<_, syn::token::Comma>>();

    let param_names = generics_params
        .iter()
        .map(|param| match param {
            syn::GenericParam::Lifetime(syn::LifetimeParam { lifetime, .. }) => {
                lifetime.to_token_stream()
            }
            syn::GenericParam::Type(type_param) => type_param.ident.to_token_stream(),
            syn::GenericParam::Const(const_param) => const_param.ident.to_token_stream(),
        })
        .collect::<syn::punctuated::Punctuated<proc_macro2::TokenStream, syn::token::Comma>>();

    Ok(quote! {
        #[allow(unused_qualifications)]
        #[automatically_derived]
        impl #lt_token #generics_params #gt_token picoserve::response::ErrorWithStatusCode for #ident #lt_token #param_names #gt_token where #where_clause_predicates {
            fn status_code(&self) -> picoserve::response::StatusCode {
                #status_code
            }
        }

        #[allow(unused_qualifications)]
        #[automatically_derived]
        impl #lt_token #generics_params #gt_token picoserve::response::IntoResponse for #ident #lt_token #param_names #gt_token where #where_clause_predicates {
            async fn write_to<R: picoserve::io::Read, W: picoserve::response::ResponseWriter<Error = R::Error>>(
                self,
                connection: picoserve::response::Connection<'_, R>,
                response_writer: W,
            ) -> Result<picoserve::ResponseSent, W::Error> {
                (picoserve::response::ErrorWithStatusCode::status_code(&self), format_args!("{self}\n"))
                    .write_to(connection, response_writer)
                    .await
            }
        }
    })
}

/// Derive `ErrorWithStatusCode` for a struct or an enum.
///
/// This will also derive `IntoResponse`, returning a `Response` with the given status code and a `text/plain` body of the `Display` implementation.
///
/// # Structs
///
/// There must be an attribute `status_code` containing the StatusCode of the error, e.g. `#[status_code(INTERNAL_SERVER_ERROR)]`.
///
/// If the `status_code` is `transparent`, the struct must contain a single field which implements `ErrorWithStatusCode`.
///
/// # Enums
///
/// There may be an attribute `status_code` on the enum itself containing the default StatusCode of the error.
///
/// There may also be an attribute `status_code` on a variant, which overrides the default StatusCode.
/// If all variants have their own attribute `status_code`, the default may be omitted.
///
/// Variants with a `status_code` of `transparent` must contain a single field which implements `ErrorWithStatusCode`.
#[proc_macro_derive(ErrorWithStatusCode, attributes(status_code))]
pub fn derive_error_with_status_code(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match try_derive_error_with_status_code(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
