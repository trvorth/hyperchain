__kernel void mine_block(
    ulong nonce_start,
    ulong target,
    __global uchar *hash_output,
    __global const uchar *block_data,
    uint block_data_len
) {
    size_t id = get_global_id(0);
    ulong nonce = nonce_start + id;

    // Simplified hash (for demo purposes)
    uchar hash[64];
    for (int i = 0; i < 64; i++) {
        hash[i] = block_data[i % block_data_len] ^ (uchar)(nonce >> (i % 8));
    }

    // Check if hash meets target
    ulong hash_prefix = 0;
    for (int i = 0; i < 8; i++) {
        hash_prefix |= ((ulong)hash[i]) << (56 - i * 8);
    }
    if (hash_prefix <= target) {
        for (int i = 0; i < 64; i++) {
            hash_output[i] = hash[i];
        }
    }
}
