from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="LogLevel")



@_attrs_define
class LogLevel:
    """ A service's level, as set from here for now.

        Attributes:
            service (str):
            level (None | str | Unset): Set for now; null is the service's configured level.
            expires_in_secs (int | None | Unset): Seconds before it goes back.
     """

    service: str
    level: None | str | Unset = UNSET
    expires_in_secs: int | None | Unset = UNSET
    additional_properties: dict[str, Any] = _attrs_field(init=False, factory=dict)





    def to_dict(self) -> dict[str, Any]:
        service = self.service

        level: None | str | Unset
        if isinstance(self.level, Unset):
            level = UNSET
        else:
            level = self.level

        expires_in_secs: int | None | Unset
        if isinstance(self.expires_in_secs, Unset):
            expires_in_secs = UNSET
        else:
            expires_in_secs = self.expires_in_secs


        field_dict: dict[str, Any] = {}
        field_dict.update(self.additional_properties)
        field_dict.update({
            "service": service,
        })
        if level is not UNSET:
            field_dict["level"] = level
        if expires_in_secs is not UNSET:
            field_dict["expires_in_secs"] = expires_in_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        service = d.pop("service")

        def _parse_level(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        level = _parse_level(d.pop("level", UNSET))


        def _parse_expires_in_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        expires_in_secs = _parse_expires_in_secs(d.pop("expires_in_secs", UNSET))


        log_level = cls(
            service=service,
            level=level,
            expires_in_secs=expires_in_secs,
        )


        log_level.additional_properties = d
        return log_level

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
