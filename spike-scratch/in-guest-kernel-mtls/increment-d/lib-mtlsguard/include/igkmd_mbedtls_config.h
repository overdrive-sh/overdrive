/*
 * Spike B (GH #303): minimal Mbed TLS 3.6.7 configuration. Only what a
 * TLS 1.3 record layer with TLS_AES_128_GCM_SHA256 needs: AES + GCM, with
 * AES-NI (inline-assembly path, runtime CPUID detection) and the built-in
 * known-answer self tests. No TLS, no X.509, no RNG, no time: the handshake
 * runs on the host.
 */
#define MBEDTLS_HAVE_ASM
#define MBEDTLS_AES_C
#define MBEDTLS_AESNI_C
#define MBEDTLS_GCM_C
#define MBEDTLS_SELF_TEST
