use util::encoding::codec;

#[codec(schema = false)]
pub struct UserTokenClaims {
    pub sub: String, // user id
    pub exp: u64,    // expiration time
    pub iat: u64,    // issued at
}
