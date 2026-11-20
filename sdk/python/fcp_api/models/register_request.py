from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset
from typing import cast






T = TypeVar("T", bound="RegisterRequest")



@_attrs_define
class RegisterRequest:
    """ `POST …/registrants` body.

        Attributes:
            name (str): The name the room shows them by (at most 64 characters).
            email (None | str | Unset):
            ttl_secs (int | None | Unset): The token's lifetime in seconds: thirty days by default, ninety at
                most.
     """

    name: str
    email: None | str | Unset = UNSET
    ttl_secs: int | None | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        name = self.name

        email: None | str | Unset
        if isinstance(self.email, Unset):
            email = UNSET
        else:
            email = self.email

        ttl_secs: int | None | Unset
        if isinstance(self.ttl_secs, Unset):
            ttl_secs = UNSET
        else:
            ttl_secs = self.ttl_secs


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "name": name,
        })
        if email is not UNSET:
            field_dict["email"] = email
        if ttl_secs is not UNSET:
            field_dict["ttl_secs"] = ttl_secs

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        name = d.pop("name")

        def _parse_email(data: object) -> None | str | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(None | str | Unset, data)

        email = _parse_email(d.pop("email", UNSET))


        def _parse_ttl_secs(data: object) -> int | None | Unset:
            if data is None:
                return data
            if isinstance(data, Unset):
                return data
            return cast(int | None | Unset, data)

        ttl_secs = _parse_ttl_secs(d.pop("ttl_secs", UNSET))


        register_request = cls(
            name=name,
            email=email,
            ttl_secs=ttl_secs,
        )

        return register_request

