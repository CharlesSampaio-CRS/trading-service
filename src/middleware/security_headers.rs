use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error,
};
use futures::future::LocalBoxFuture;
use std::future::{ready, Ready};

pub struct SecurityHeaders;

impl<S, B> Transform<S, ServiceRequest> for SecurityHeaders
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = SecurityHeadersMiddleware<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(SecurityHeadersMiddleware { service }))
    }
}

pub struct SecurityHeadersMiddleware<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for SecurityHeadersMiddleware<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let fut = self.service.call(req);

        Box::pin(async move {
            let mut res = fut.await?;

            // Adiciona headers para permitir comunicação cross-origin entre janelas
            // Isso é necessário para OAuth popup funcionar
            let headers = res.headers_mut();
            
            // Permite acesso ao popup sem restrições (necessário para OAuth)
            headers.insert(
                actix_web::http::header::HeaderName::from_static("cross-origin-opener-policy"),
                actix_web::http::header::HeaderValue::from_static("unsafe-none"),
            );
            
            // Permite embeddings cross-origin
            headers.insert(
                actix_web::http::header::HeaderName::from_static("cross-origin-embedder-policy"),
                actix_web::http::header::HeaderValue::from_static("unsafe-none"),
            );

            Ok(res)
        })
    }
}
