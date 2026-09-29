/*
 * Spike B2 (GH #303, increment-e): minimal Mbed TLS 3.6.7 configuration.
 * A TLS 1.3 record layer with TLS_AES_128_GCM_SHA256 (AES + GCM with AES-NI)
 * plus the key schedule a KeyUpdate needs: HKDF-Expand-Label over SHA-256
 * (MD + SHA256 + HKDF). No TLS, no X.509, no RNG, no time: the handshake runs
 * on the host. Built-in known-answer self tests enabled.
 */
#define MBEDTLS_HAVE_ASM
#define MBEDTLS_AES_C
#define MBEDTLS_AESNI_C
#define MBEDTLS_GCM_C
#define MBEDTLS_MD_C
#define MBEDTLS_SHA256_C
#define MBEDTLS_HKDF_C
#define MBEDTLS_SELF_TEST
