from __future__ import annotations

from collections.abc import Mapping
from typing import Any, TypeVar, BinaryIO, TextIO, TYPE_CHECKING, Generator

from attrs import define as _attrs_define
from attrs import field as _attrs_field

from ..types import UNSET, Unset

from ..types import UNSET, Unset






T = TypeVar("T", bound="SamlSettings")



@_attrs_define
class SamlSettings:
    """ A SAML identity provider, from its metadata.

        Attributes:
            idp_metadata_xml (str): The provider's metadata XML, as it publishes it: its entity id, its
                single sign-on address and the certificates it signs with.
            trust_email (bool | Unset): Take the email attribute as the person's own, for linking by
                email. SAML has no `email_verified`; turn this on only for a
                provider whose directory owns its users' addresses.
     """

    idp_metadata_xml: str
    trust_email: bool | Unset = UNSET





    def to_dict(self) -> dict[str, Any]:
        idp_metadata_xml = self.idp_metadata_xml

        trust_email = self.trust_email


        field_dict: dict[str, Any] = {}

        field_dict.update({
            "idp_metadata_xml": idp_metadata_xml,
        })
        if trust_email is not UNSET:
            field_dict["trust_email"] = trust_email

        return field_dict



    @classmethod
    def from_dict(cls: type[T], src_dict: Mapping[str, Any]) -> T:
        d = dict(src_dict)
        idp_metadata_xml = d.pop("idp_metadata_xml")

        trust_email = d.pop("trust_email", UNSET)

        saml_settings = cls(
            idp_metadata_xml=idp_metadata_xml,
            trust_email=trust_email,
        )

        return saml_settings

