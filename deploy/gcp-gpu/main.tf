# One GPU development box for FCP video conferencing phase 8 (the GPU
# backend), brought up for a session and torn down after it.
#
# The boot disk is its own resource and survives the VM: `terraform apply
# -var up=false` destroys the VM and nothing else, so the driver, CUDA,
# the Rust toolchain, the checkout and the cargo target directory are
# still there next time and the VM is back in the time it takes to boot.
# The disk is never destroyed by Terraform; delete it by hand when the
# work is over.

resource "google_project_service" "iap" {
  service            = "iap.googleapis.com"
  disable_on_destroy = false
}

# The disk: Debian 12, grown to hold a toolchain and a build cache.
resource "google_compute_disk" "boot" {
  name  = var.name
  zone  = var.zone
  type  = "pd-balanced"
  size  = var.disk_size_gb
  image = "debian-cloud/debian-12"

  lifecycle {
    prevent_destroy = true
    # A disk moved to another zone (`gpu-box move`) comes back from a
    # snapshot, not the image: that is not a reason to replace it.
    ignore_changes = [image, snapshot]
  }
}

# SSH from IAP's forwarding range only: no public port 22.
resource "google_compute_firewall" "iap_ssh" {
  name    = "${var.name}-iap-ssh"
  network = "default"

  allow {
    protocol = "tcp"
    ports    = ["22"]
  }

  source_ranges = ["35.235.240.0/20"]
  target_tags   = [var.name]
}

resource "google_compute_instance" "gpu" {
  count = var.up ? 1 : 0

  name         = var.name
  machine_type = var.machine_type
  zone         = var.zone
  tags         = [var.name]

  boot_disk {
    source      = google_compute_disk.boot.self_link
    auto_delete = false
  }

  guest_accelerator {
    type  = "nvidia-l4"
    count = 1
  }

  scheduling {
    on_host_maintenance         = "TERMINATE"
    automatic_restart           = false
    provisioning_model          = var.spot ? "SPOT" : "STANDARD"
    preemptible                 = var.spot
    instance_termination_action = var.spot ? "STOP" : null
  }

  network_interface {
    network = "default"
    # An ephemeral external address so the box can reach package
    # mirrors, crates.io and GitHub; nothing listens on it.
    access_config {}
  }

  metadata = {
    enable-oslogin = "TRUE"
    startup-script = file("${path.module}/scripts/startup.sh")
  }

  service_account {
    scopes = ["https://www.googleapis.com/auth/logging.write", "https://www.googleapis.com/auth/monitoring.write"]
  }

  # Preempted or stopped by hand: `terraform apply` brings it back.
  allow_stopping_for_update = true

  depends_on = [google_project_service.iap]
}
