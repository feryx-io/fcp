from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="ClaimMapping")



@_attrs_define
class ClaimMapping:
    """ Which claims become the user's names.

        Attributes:
            username (str | Unset): The username of a user made just in time; the part before the `@`
                when the claim is an address. Falls back to the email's.
            email (str | Unset):
            display_name (str | Unset):
            first_name (str | Unset): The first name (#343); the well-known given-name claims and SAML
                attributes are read when this one is missing.
            last_name (str | Unset): The last name; the well-known family-name claims and SAML
                attributes are read when this one is missing.
            groups (str | Unset): A list of strings: the user's groups (Entra ID sends ids, Okta
                and Google Workspace names, when asked to).
     """

    username: str | Unset = UNSET
    email: str | Unset = UNSET
    display_name: str | Unset = UNSET
    first_name: str | Unset = UNSET
    last_name: str | Unset = UNSET
    groups: str | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        username = self.username

        email = self.email

        display_name = self.display_name

        first_name = self.first_name

        last_name = self.last_name

        groups = self.groups


        field_dict: dict[str, Any] = {}

        field_dict.update({
        })
        if username is not UNSET:
            field_dict["username"] = username
        if email is not UNSET:
            field_dict["email"] = email
        if display_name is not UNSET:
            field_dict["display_name"] = display_name
        if first_name is not UNSET:
            field_dict["first_name"] = first_name
        if last_name is not UNSET:
            field_dict["last_name"] = last_name
        if groups is not UNSET:
            field_dict["groups"] = groups

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        username = d.pop("username", UNSET)

        email = d.pop("email", UNSET)

        display_name = d.pop("display_name", UNSET)

        first_name = d.pop("first_name", UNSET)

        last_name = d.pop("last_name", UNSET)

        groups = d.pop("groups", UNSET)

        claim_mapping = cls(
            username=username,
            email=email,
            display_name=display_name,
            first_name=first_name,
            last_name=last_name,
            groups=groups,
        )

        return claim_mapping

