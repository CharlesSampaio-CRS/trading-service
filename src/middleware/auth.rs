use actix_web::{
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage,
};
use futures::future::LocalBoxFuture;
use std::future::{ready, Ready};
use serde::{Deserialize, Serialize};

// Re-export Claims from auth_service to keep consistency
pub use crate::services::auth_service::Claims;

pub struct AuthMiddleware;

impl<S, B> Transform<S, ServiceRequest> for AuthMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = AuthMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(AuthMiddlewareService { service }))
    }
}

pub struct AuthMiddlewareService<S> {
    service: S,
}

impl<S, B> Service<ServiceRequest> for AuthMiddlewareService<S>
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
        let path = req.path().to_string();
        
        // Skip auth for health check
        if path == "/health" {
            let fut = self.service.call(req);
            return Box::pin(async move {
                let res = fut.await?;
                Ok(res)
            });
        }
        
        // Get Authorization header
        let auth_header = req.headers().get("Authorization");
        
        match auth_header {
            Some(header_value) => {
                if let Ok(header_str) = header_value.to_str() {
                    if header_str.starts_with("Bearer ") {
                        let token = &header_str[7..];
                        
                        // Verify and decode JWT token
                        match crate::services::auth_service::verify_token(token) {
                            Ok(claims) => {
                                // Insert Claims into request extensions so handlers can access it
                                req.extensions_mut().insert(claims);
                                
                                let fut = self.service.call(req);
                                return Box::pin(async move {
                                    let res = fut.await?;
                                    Ok(res)
                                });
                            }
                            Err(e) => {
                                log::warn!("🔒 Invalid JWT token: {}", e);
                                return Box::pin(async move {
                                    Err(actix_web::error::ErrorUnauthorized(format!("Invalid token: {}", e)))
                                });
                            }
                        }
                    }
                }
                
                Box::pin(async move {
                    Err(actix_web::error::ErrorUnauthorized("Invalid token format"))
                })
            }
            None => {
                Box::pin(async move {
                    Err(actix_web::error::ErrorUnauthorized("Missing authorization token"))
                })
            }
        }
    }
}
