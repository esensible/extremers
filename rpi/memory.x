MEMORY {
    BOOT2 : ORIGIN = 0x10000000, LENGTH = 0x100
    # artificially small so I can track flash usage
    FLASH : ORIGIN = 0x10000100, LENGTH = 544K - 0x100
    STORAGE: ORIGIN = 0x10100000, LENGTH = 0x100000
    RAM   : ORIGIN = 0x20000000, LENGTH = 256K
}

__storage_base = ORIGIN(STORAGE);
__storage_size = LENGTH(STORAGE);