package stormchaser

default allow := false

# Allow all requests for local development
allow := true if {
    true
}
