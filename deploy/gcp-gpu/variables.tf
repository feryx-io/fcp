variable "project" {
  description = "The GCP project the box lives in."
  type        = string
  default     = "brayz-dev"
}

variable "region" {
  description = "Region; us-central1 offers the L4 in three zones."
  type        = string
  default     = "us-central1"
}

variable "zone" {
  description = "Zone for the VM and its disk. The disk is zonal: to follow Spot capacity to another zone, `gpu-box move <zone>` snapshots it there and this default (or a zone.auto.tfvars) is updated."
  type        = string
  default     = "us-central1-b"
}

variable "up" {
  description = "Whether the VM exists. `false` tears the VM down and keeps the disk: `terraform apply -var up=false`."
  type        = bool
  default     = true
}

variable "machine_type" {
  description = "One NVIDIA L4 comes with g2-standard-4 (4 vCPU, 16 GB); g2-standard-8 doubles the CPU for the same GPU."
  type        = string
  default     = "g2-standard-4"
}

variable "spot" {
  description = "Run as a Spot VM (about a third of the on-demand price; can be preempted, and everything lives on the disk)."
  type        = bool
  default     = true
}

variable "disk_size_gb" {
  description = "The persistent boot disk that holds the OS, driver, toolchain, checkout and build cache."
  type        = number
  default     = 200
}

variable "name" {
  description = "Name of the VM, disk and firewall rule."
  type        = string
  default     = "fcp-gpu"
}
