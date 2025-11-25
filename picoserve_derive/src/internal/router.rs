use proc_macro2::TokenStream;

struct MethodInfo {
    ident: syn::Ident,
    type_parameter: syn::Ident,
    method_name: String,
}

struct MethodInfoList(Vec<MethodInfo>);

impl core::ops::Deref for MethodInfoList {
    type Target = [MethodInfo];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl syn::parse::Parse for MethodInfoList {
    fn parse(input: syn::parse::ParseStream) -> syn::Result<Self> {
        input
            .parse_terminated(syn::Ident::parse, syn::token::Comma)
            .map(|list| {
                Self(
                    list.into_iter()
                        .map(|ident| {
                            let method_name = ident.to_string().to_uppercase();
                            let type_parameter = syn::Ident::new(&method_name, ident.span());

                            MethodInfo {
                                ident,
                                type_parameter,
                                method_name,
                            }
                        })
                        .collect(),
                )
            })
    }
}

struct FunctionCodeParts {
    ident: syn::Ident,
    additional_generics: TokenStream,
    parameters: TokenStream,
    field_value: TokenStream,
}

impl FunctionCodeParts {
    fn for_function_handler(method_info: &MethodInfo) -> Self {
        Self {
            ident: method_info.ident.clone(),
            additional_generics: quote::quote! { T, Handler: RequestHandlerFunction<State, PathParameters, T> },
            parameters: quote::quote! { handler: Handler },
            field_value: quote::quote! { HandlerFunctionRequestHandler::new(handler) },
        }
    }

    fn for_service_handler(method_info: &MethodInfo) -> Self {
        Self {
            ident: syn::Ident::new(
                &format!("{}_service", method_info.ident),
                method_info.ident.span(),
            ),
            additional_generics: quote::quote! {},
            parameters: quote::quote! { service: impl RequestHandlerService<State, PathParameters> },
            field_value: quote::quote! { RequestHandlerServiceRequestHandler { service } },
        }
    }
}

impl MethodInfo {
    fn generate_function(
        &self,
        method_index: usize,
        all_method_info: &[Self],
        doc_comment: String,
        FunctionCodeParts {
            ident,
            additional_generics,
            parameters,
            field_value,
        }: FunctionCodeParts,
    ) -> TokenStream {
        let method_not_allowed_list =
            std::iter::from_fn(|| Some(quote::quote! { MethodNotAllowed }));

        let method_not_allowed_before = method_not_allowed_list.clone().take(method_index);
        let method_not_allowed_after = method_not_allowed_list
            .take(all_method_info.len())
            .skip(method_index + 1);

        let fields = all_method_info
            .iter()
            .enumerate()
            .map(|(index, other_method_info)| {
                let field_value = if index == method_index {
                    field_value.clone()
                } else {
                    quote::quote! { MethodNotAllowed }
                };

                let field_ident = &other_method_info.ident;

                quote::quote! { #field_ident: #field_value }
            });

        quote::quote! {
            #[doc = #doc_comment]
            pub fn #ident<State, PathParameters, #additional_generics>(
                #parameters
            ) -> MethodRouter<
                #(#method_not_allowed_before,)*
                impl RequestHandler<State, PathParameters>,
                #(#method_not_allowed_after,)*
            > {
                MethodRouter {
                    #(#fields,)*
                }
            }
        }
    }

    fn generate_functions(&self, method_index: usize, all_method_info: &[Self]) -> TokenStream {
        let doc_comment_suffix = if self.method_name == "GET" {
            " Also routes `HEAD` requests by simply discarding the response body after routing the request."
        } else {
            ""
        };

        let function_handler = self.generate_function(
            method_index,
            all_method_info,
            format!(
                "Route `{}` requests to the given [handler](RequestHandlerFunction).{}",
                self.type_parameter, doc_comment_suffix
            ),
            FunctionCodeParts::for_function_handler(self),
        );

        let service_handler = self.generate_function(
            method_index,
            all_method_info,
            format!(
                "Route `{}` requests to the given [service](RequestHandlerService).{}",
                self.type_parameter, doc_comment_suffix
            ),
            FunctionCodeParts::for_service_handler(self),
        );

        quote::quote! {
            #function_handler
            #service_handler
        }
    }

    fn generate_method(
        &self,
        method_index: usize,
        all_method_info: &[Self],
        doc_comment: String,
        FunctionCodeParts {
            ident,
            additional_generics,
            parameters,
            field_value,
        }: FunctionCodeParts,
    ) -> TokenStream {
        let return_type_parameters =
            all_method_info
                .iter()
                .enumerate()
                .map(|(index, other_method_info)| {
                    if index == method_index {
                        quote::quote! { impl RequestHandler<State, PathParameters> }
                    } else {
                        let type_parameter = &other_method_info.type_parameter;

                        quote::quote! { #type_parameter }
                    }
                });

        let unpack_fields = all_method_info
            .iter()
            .enumerate()
            .map(|(index, other_method_info)| {
                let field_ident = &other_method_info.ident;

                if index == method_index {
                    quote::quote! { #field_ident: MethodNotAllowed }
                } else {
                    quote::quote! { #field_ident }
                }
            });

        let fields = all_method_info
            .iter()
            .enumerate()
            .map(|(index, other_method_info)| {
                let field_ident = &other_method_info.ident;

                if index == method_index {
                    let field_value = field_value.clone();
                    quote::quote! { #field_ident: #field_value }
                } else {
                    quote::quote! { #field_ident }
                }
            });

        quote::quote! {
            #[doc = #doc_comment]
            pub fn #ident<State, PathParameters, #additional_generics>(
                self,
                #parameters
            ) -> MethodRouter<
                #(#return_type_parameters,)*
            > {
                let MethodRouter {
                    #(#unpack_fields,)*
                } = self;

                MethodRouter {
                    #(#fields,)*
                }
            }
        }
    }

    fn generate_methods(&self, method_index: usize, all_method_info: &[Self]) -> TokenStream {
        let doc_comment_suffix = if self.method_name == "GET" {
            " Also routes `HEAD` requests by simply discarding the response body after routing the request."
        } else {
            ""
        };

        let type_parameter_declarations =
            all_method_info
                .iter()
                .enumerate()
                .filter_map(|(index, other_method_info)| {
                    (index != method_index).then_some(&other_method_info.type_parameter)
                });

        let type_parameters =
            all_method_info
                .iter()
                .enumerate()
                .map(|(index, MethodInfo { type_parameter, .. })| {
                    if index == method_index {
                        quote::quote! { MethodNotAllowed }
                    } else {
                        quote::quote! { #type_parameter }
                    }
                });

        let function_handler = self.generate_method(
            method_index,
            all_method_info,
            format!(
                "Chain an additional [handler](RequestHandlerFunction) that will only accept `{}` requests.{}",
                self.type_parameter, doc_comment_suffix
            ),
            FunctionCodeParts::for_function_handler(self),
        );

        let service_handler = self.generate_method(
            method_index,
            all_method_info,
            format!(
                "Chain an additional [service](RequestHandlerService) that will only accept `{}` requests.{}",
                self.type_parameter, doc_comment_suffix
            ),
            FunctionCodeParts::for_service_handler(self),
        );

        quote::quote! {
            impl<#(#type_parameter_declarations,)*> MethodRouter<#(#type_parameters,)*> {
                #function_handler
                #service_handler
            }
        }
    }
}

impl MethodInfoList {
    fn generate_method_router_struct(&self) -> proc_macro2::TokenStream {
        let struct_doc_comment = r#"
A [MethodHandler] which routes requests to the appropriate [RequestHandler] based on the method.

Automatically handled the `HEAD` method by calling the `GET` handler and returning an empty body.
    "#;

        let type_parameter_declarations = self
            .iter()
            .map(|MethodInfo { type_parameter, .. }| type_parameter);

        let field_declarations = self.iter().map(
            |MethodInfo {
                 ident,
                 type_parameter,
                 ..
             }| {
                quote::quote! { #ident: #type_parameter }
            },
        );

        quote::quote! {
            #[doc = #struct_doc_comment]
            pub struct MethodRouter<#(#type_parameter_declarations,)*> {
                #(#field_declarations,)*
            }
        }
    }

    fn generate_layer_method(&self) -> TokenStream {
        let type_parameter_declarations = self
            .iter()
            .map(|MethodInfo { type_parameter, .. }| type_parameter)
            .collect::<Vec<_>>();

        let type_bindings = type_parameter_declarations.iter().map(
            |type_parameter| quote::quote! { #type_parameter: RequestHandler<L::NextState, L::NextPathParameters> },
        );

        quote::quote! {
            impl<#(#type_parameter_declarations,)*> MethodRouter<#(#type_parameter_declarations,)*>
            {
                #[doc = "Add a [Layer] to all routes in the router"]
                pub fn layer<State, PathParameters, L: Layer<State, PathParameters>>(
                    self,
                    layer: L,
                ) -> impl MethodHandler<State, PathParameters>
                where
                    #(#type_bindings,)*
                {
                    layer::MethodRouterLayer { layer, inner: self }
                }
            }
        }
    }

    fn generate_impl_method_handler(&self) -> proc_macro2::TokenStream {
        let type_parameter_declarations = self
            .iter()
            .map(|MethodInfo { type_parameter, .. }| type_parameter)
            .collect::<Vec<_>>();

        let type_bindings = type_parameter_declarations.iter().map(
            |type_parameter| quote::quote! { #type_parameter: RequestHandler<State, PathParameters> },
        );

        let match_cases = self.iter().map(
            |MethodInfo {
                 ident, method_name, ..
             }| {
                quote::quote! {
                    #method_name => {
                        self.#ident
                            .call_request_handler(state, path_parameters, request, response_writer)
                            .await
                    }
                }
            },
        );

        quote::quote! {
            impl<#(#type_parameter_declarations,)*> sealed::MethodHandlerIsSealed for MethodRouter<#(#type_parameter_declarations,)*> {}

            impl<
                    State,
                    PathParameters,
                    #(#type_bindings,)*
                > MethodHandler<State, PathParameters>
                for MethodRouter<#(#type_parameter_declarations,)*>
            {
                async fn call_method_handler<R: Read, W: ResponseWriter<Error = R::Error>>(
                    &self,
                    state: &State,
                    path_parameters: PathParameters,
                    request: Request<'_, R>,
                    response_writer: W,
                ) -> Result<ResponseSent, W::Error> {
                    match request.parts.method() {
                        #(#match_cases)*
                        "HEAD" => {
                            self.get
                                .call_request_handler(
                                    state,
                                    path_parameters,
                                    request,
                                    head_method_util::ignore_body(response_writer),
                                )
                                .await
                        }
                        _ => {
                            MethodNotAllowed
                                .call_request_handler(state, path_parameters, request, response_writer)
                                .await
                        }
                    }
                }
            }
        }
    }
}

pub(crate) fn generate_method_router(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    let method_info_list = syn::parse_macro_input!(input as MethodInfoList);

    let struct_declaration = method_info_list.generate_method_router_struct();

    let functions = method_info_list
        .iter()
        .enumerate()
        .map(|(method_index, method_info)| {
            method_info.generate_functions(method_index, &method_info_list)
        });

    let methods = method_info_list
        .iter()
        .enumerate()
        .map(|(method_index, method_info)| {
            method_info.generate_methods(method_index, &method_info_list)
        });

    let layer_method = method_info_list.generate_layer_method();

    let impl_method_handler = method_info_list.generate_impl_method_handler();

    quote::quote! {
        #struct_declaration

        #(#functions)*
        #(#methods)*

        #layer_method

        #impl_method_handler

    }
    .into()
}

// impl <#(#type_parameter_declarations,)*> sealed::MethodHandlerIsSealed for MethodRouter<#(#type_parameter_declarations,)*> {}

/*


*/
