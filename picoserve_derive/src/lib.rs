use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

fn single_field(fields: &syn::Fields) -> Option<proc_macro2::TokenStream> {
    match fields {
        syn::Fields::Named(fields) => {
            let mut fields = fields.named.iter();
            let field = fields.next()?;
            fields
                .next()
                .is_none()
                .then(|| quote! { { #field: field } })
        }
        syn::Fields::Unnamed(fields) => {
            let mut fields = fields.unnamed.iter();
            let _field = fields.next()?;
            fields.next().is_none().then(|| quote! { (field) })
        }
        syn::Fields::Unit => None,
    }
}

enum StatusCodeAttr {
    StatusCode(proc_macro2::TokenStream),
    Transparent,
}

fn status_code_attr(attrs: &[syn::Attribute]) -> Result<Option<StatusCodeAttr>, &'static str> {
    let mut attrs = attrs.iter();

    loop {
        let Some(attr) = attrs.next() else {
            return Ok(None);
        };

        if !attr.path().is_ident("status_code") {
            continue;
        };

        let syn::Meta::List(meta_list) = &attr.meta else {
            return Err("#[status_code(...)] must be a `StatusCode`");
        };

        let status_code = &meta_list.tokens;

        return Ok(Some(

        if status_code.to_string() == "transparent" {
            StatusCodeAttr::Transparent
        } else {

        StatusCodeAttr::StatusCode(
            quote! { picoserve::response::StatusCode::#status_code },
        )}));
    }
}

fn try_derive_error_with_status_code(
    input: &DeriveInput,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let ident = &input.ident;

    let default_status_code = status_code_attr(&input.attrs)
        .map_err(|message| syn::Error::new_spanned(input, message))?;

    let status_code = match &input.data {
        syn::Data::Struct(data_struct) => match default_status_code
            .ok_or_else(|| syn::Error::new_spanned(input, "missing #[status_code(..)]"))?
        {
            StatusCodeAttr::StatusCode(token_stream) => token_stream,
            StatusCodeAttr::Transparent => {
                let fields = single_field(&data_struct.fields).ok_or_else(|| {
                    syn::Error::new_spanned(
                        input,
                        "transparent errors must have a single field",
                    )
                })?;

                quote! {
                    let Self #fields = self;
                    picoserve::response::ErrorWithStatusCode::status_code(field)
                }
            },
        }
        syn::Data::Enum(data_enum) => {
            let cases = data_enum
                .variants
                .iter()
                .map(|variant| {
                    let variant_status_code = status_code_attr(&variant.attrs)
                        .map_err(|message| syn::Error::new_spanned(ident, message))?;

                    let selected_status_code = variant_status_code.as_ref().or(default_status_code.as_ref()).ok_or_else(|| {
                        syn::Error::new_spanned(
                            variant,
                            "transparent errors must have a single field",
                        )
                    })?;

                    let ident = &variant.ident;
                    let fields;
                    let status_code;

                    match selected_status_code {
                        StatusCodeAttr::StatusCode(selected_status_code) => {
                            fields = match variant.fields {
                                syn::Fields::Named(..) => quote! { {..} },
                                syn::Fields::Unnamed(..) => quote! { (..) },
                                syn::Fields::Unit => quote! {},
                            };

                            status_code = selected_status_code.clone();
                        },
                        StatusCodeAttr::Transparent => {
                            fields = single_field(&variant.fields).ok_or_else(|| {
                                syn::Error::new_spanned(
                                    variant,
                                    "transparent errors must have a single field",
                                )
                            })?;
    
                            status_code = quote! { picoserve::response::ErrorWithStatusCode::status_code(field) };
                        },
                    }

      

                    Ok(quote! { Self::#ident #fields => #status_code, })
                })
                .collect::<Result<proc_macro2::TokenStream, syn::Error>>()?;

            quote! {
                match self {
                    #cases
                }
            }
        }
        syn::Data::Union(..) => {
            return Err(syn::Error::new_spanned(
                input,
                "Must Be a struct or an enum",
            ))
        }
    };

    Ok(quote! {
        impl picoserve::response::ErrorWithStatusCode for #ident {
            fn status_code(&self) -> picoserve::response::StatusCode {
                #status_code
            }
        }

        impl picoserve::response::IntoResponse for #ident {
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
/// Variants with a `status_code` of transparent must contain a single field which implements `ErrorWithStatusCode`.
#[proc_macro_derive(ErrorWithStatusCode, attributes(status_code))]
pub fn derive_error_with_status_code(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);

    match try_derive_error_with_status_code(&input) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.into_compile_error().into(),
    }
}
