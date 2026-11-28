from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from typing import cast






T = TypeVar("T", bound="RestoreCapabilities")



@_attrs_define
class RestoreCapabilities:
    """ What restores the backup service may do: switched on by
    `docker-compose.restore.yml`.

        Attributes:
            service (bool): The backup service is set up here at all.
            file_restores (bool): Its volumes are writable: files can be put back.
            full_restore (bool): It has the Docker socket: the whole system can be restored.
            read_only (list[str]): The volumes still read-only.
     """

    service: bool
    file_restores: bool
    full_restore: bool
    read_only: list[str]
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        service = self.service

        file_restores = self.file_restores

        full_restore = self.full_restore

        read_only = self.read_only




        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "service": service,
            "file_restores": file_restores,
            "full_restore": full_restore,
            "read_only": read_only,
        })

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        service = d.pop("service")

        file_restores = d.pop("file_restores")

        full_restore = d.pop("full_restore")

        read_only = cast(list[str], d.pop("read_only"))


        restore_capabilities = cls(
            service=service,
            file_restores=file_restores,
            full_restore=full_restore,
            read_only=read_only,
        )


        restore_capabilities.additional_properties = d
        return restore_capabilities

    @property
    def additional_keys(self) -> list[str]:
        return list(self.additional_properties.keys())

    def __getitem__(self, key: str) -> Any:
        return self.additional_properties[key]

    def __setitem__(self, key: str, value: Any) -> None:
        self.additional_properties[key] = value

    def __delitem__(self, key: str) -> None:
        del self.additional_properties[key]

    def __contains__(self, key: str) -> bool:
        return key in self.additional_properties
