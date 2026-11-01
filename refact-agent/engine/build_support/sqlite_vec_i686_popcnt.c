unsigned long long __popcnt64(unsigned long long value) {
    unsigned int count = 0;
    while (value != 0) {
        value &= value - 1;
        ++count;
    }
    return count;
}
